# MacroFlow area selector for Windows, modelled on the Snipping Tool (Win+Shift+S):
# the screen freezes and dims, a small toolbar sits at the top centre, you drag a
# rectangle (shown at full brightness with a thin border and its size), and letting
# go confirms straight away. Esc or the toolbar's close button cancels.
#
#   powershell -File select.ps1 -ImagePath shot.png -Device \\.\DISPLAY1 -X 0 -Y 0 -W 1920 -H 1080 [-Hint text]
#   powershell -File select.ps1 -Warmup      (just compile and cache the window code)
#
# The overlay covers the display named by -Device (falling back to X/Y/W/H, then the
# display under the mouse). It prints "OK x0 y0 x1 y1": the box as fractions (0-1) of
# that display, so DPI scaling can't throw the numbers off.
# Exit codes: 0 = picked, 3 = cancelled, 2 = error (details on stderr).
# MacroFlow embeds this script and runs it automatically.
# (Keep this file plain ASCII: Windows PowerShell 5.1 reads scripts as ANSI.)

param(
    [string]$ImagePath = '',
    [string]$Device = '',
    [int]$X = 0,
    [int]$Y = 0,
    [int]$W = 0,
    [int]$H = 0,
    [string]$Hint = 'Drag to select an area',
    [switch]$Warmup
)

$ErrorActionPreference = 'Stop'
$logPath = Join-Path $env:TEMP 'macroflow-select.log'

# C# 5 only (Windows PowerShell compiles with the old compiler): no $"", ?., => members.
$source = @'
using System;
using System.Globalization;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Drawing.Text;
using System.Runtime.InteropServices;
using System.Windows.Forms;

public static class MacroFlowSnipNative
{
    [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr value);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool AllowSetForegroundWindow(int processId);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, IntPtr processId);
    [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
    [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint idAttach, uint idAttachTo, bool fAttach);
    [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern short GetAsyncKeyState(int vKey);

    /// Windows only lets the app you're using bring a window to the front. Briefly
    /// sharing input with the current foreground window gets round that, so the
    /// overlay gets keyboard focus (for Esc) instead of just flashing in the taskbar.
    public static void ForceForeground(IntPtr hWnd)
    {
        IntPtr fg = GetForegroundWindow();
        uint fgThread = fg == IntPtr.Zero ? 0 : GetWindowThreadProcessId(fg, IntPtr.Zero);
        uint me = GetCurrentThreadId();
        bool attached = false;
        if (fgThread != 0 && fgThread != me) attached = AttachThreadInput(fgThread, me, true);
        BringWindowToTop(hWnd);
        SetForegroundWindow(hWnd);
        if (attached) AttachThreadInput(fgThread, me, false);
    }

    public static bool EscapeDown()
    {
        return (GetAsyncKeyState(0x1B) & 0x8000) != 0;
    }

    public static void MakeDpiAware()
    {
        try
        {
            // Per-monitor v2, so every coordinate is in real screen pixels.
            if (SetProcessDpiAwarenessContext(new IntPtr(-4))) return;
        }
        catch (Exception) { }
        try { SetProcessDPIAware(); } catch (Exception) { }
    }
}

public class MacroFlowSnipForm : Form
{
    private readonly Bitmap shot;
    private readonly string hint;
    private Point start;
    private Point current;
    private bool dragging;
    private bool hoverClose;
    private readonly Timer watch = new Timer();
    private bool escWasDown;
    public string Result;

    public MacroFlowSnipForm(Bitmap shot, Rectangle bounds, string hint)
    {
        this.shot = shot;
        this.hint = hint;
        FormBorderStyle = FormBorderStyle.None;
        StartPosition = FormStartPosition.Manual;
        Bounds = bounds;
        TopMost = true;
        ShowInTaskbar = false;
        KeyPreview = true;
        BackColor = Color.Black;
        Cursor = Cursors.Cross;
        Text = "MacroFlow area select";
        SetStyle(ControlStyles.AllPaintingInWmPaint | ControlStyles.UserPaint |
                 ControlStyles.OptimizedDoubleBuffer | ControlStyles.ResizeRedraw, true);
        DoubleBuffered = true;

        // Esc works even if Windows didn't give this window keyboard focus, and the
        // window keeps itself on top in case something else jumps in front of it.
        escWasDown = MacroFlowSnipNative.EscapeDown();
        watch.Interval = 50;
        watch.Tick += delegate
        {
            bool down = MacroFlowSnipNative.EscapeDown();
            if (down && !escWasDown) { Finish(null); return; }
            escWasDown = down;
            if (!dragging && Form.ActiveForm != this) { TopMost = true; BringToFront(); }
        };
    }

    private Rectangle Toolbar
    {
        get { return new Rectangle((ClientSize.Width - 132) / 2, 18, 132, 44); }
    }
    private Rectangle RectButton
    {
        get { Rectangle t = Toolbar; return new Rectangle(t.X + 6, t.Y + 6, 32, 32); }
    }
    private Rectangle CloseButton
    {
        get { Rectangle t = Toolbar; return new Rectangle(t.Right - 38, t.Y + 6, 32, 32); }
    }

    private Rectangle Selection
    {
        get
        {
            int x0 = Math.Min(start.X, current.X), y0 = Math.Min(start.Y, current.Y);
            int x1 = Math.Max(start.X, current.X), y1 = Math.Max(start.Y, current.Y);
            return Rectangle.FromLTRB(x0, y0, x1, y1);
        }
    }

    private bool HasSelection
    {
        get { Rectangle s = Selection; return dragging && s.Width >= 1 && s.Height >= 1; }
    }

    protected override void OnShown(EventArgs e)
    {
        base.OnShown(e);
        MacroFlowSnipNative.ForceForeground(Handle);
        Activate();
        Focus();
        watch.Start();
    }

    private static GraphicsPath Rounded(Rectangle r, int radius)
    {
        GraphicsPath p = new GraphicsPath();
        int d = radius * 2;
        p.AddArc(r.X, r.Y, d, d, 180, 90);
        p.AddArc(r.Right - d, r.Y, d, d, 270, 90);
        p.AddArc(r.Right - d, r.Bottom - d, d, d, 0, 90);
        p.AddArc(r.X, r.Bottom - d, d, d, 90, 90);
        p.CloseFigure();
        return p;
    }

    protected override void OnPaint(PaintEventArgs e)
    {
        Graphics g = e.Graphics;
        g.InterpolationMode = InterpolationMode.HighQualityBilinear;
        g.DrawImage(shot, new Rectangle(0, 0, ClientSize.Width, ClientSize.Height));

        // Dim everything except the selection.
        using (Region dim = new Region(ClientRectangle))
        using (SolidBrush shade = new SolidBrush(Color.FromArgb(128, 0, 0, 0)))
        {
            if (HasSelection) dim.Exclude(Selection);
            g.FillRegion(shade, dim);
        }

        g.SmoothingMode = SmoothingMode.AntiAlias;
        g.TextRenderingHint = TextRenderingHint.ClearTypeGridFit;

        if (HasSelection)
        {
            Rectangle s = Selection;
            using (Pen border = new Pen(Color.White, 1))
            {
                g.SmoothingMode = SmoothingMode.None;
                g.DrawRectangle(border, s.X, s.Y, s.Width - 1, s.Height - 1);
                g.SmoothingMode = SmoothingMode.AntiAlias;
            }
            DrawLabel(g, string.Format("{0} \u00D7 {1}", s.Width, s.Height), s);
        }
        else
        {
            DrawHint(g);
        }
        DrawToolbar(g);
    }

    private void DrawLabel(Graphics g, string text, Rectangle s)
    {
        using (Font f = new Font("Segoe UI", 9f, FontStyle.Regular))
        {
            SizeF size = g.MeasureString(text, f);
            int w = (int)size.Width + 10, h = (int)size.Height + 4;
            int x = s.X, y = s.Bottom + 6;
            if (y + h > ClientSize.Height) y = s.Y - h - 6;
            if (y < 0) { x = s.X + 6; y = s.Y + 6; }
            Rectangle box = new Rectangle(x, y, w, h);
            using (GraphicsPath p = Rounded(box, 4))
            using (SolidBrush b = new SolidBrush(Color.FromArgb(230, 32, 32, 32)))
                g.FillPath(b, p);
            g.DrawString(text, f, Brushes.White, x + 5, y + 2);
        }
    }

    private void DrawHint(Graphics g)
    {
        using (Font f = new Font("Segoe UI", 10f, FontStyle.Regular))
        {
            SizeF size = g.MeasureString(hint, f);
            int w = (int)size.Width + 20, h = (int)size.Height + 8;
            Rectangle box = new Rectangle((ClientSize.Width - w) / 2, Toolbar.Bottom + 10, w, h);
            using (GraphicsPath p = Rounded(box, 6))
            using (SolidBrush b = new SolidBrush(Color.FromArgb(190, 24, 24, 24)))
                g.FillPath(b, p);
            g.DrawString(hint, f, Brushes.White, box.X + 10, box.Y + 4);
        }
    }

    private void DrawToolbar(Graphics g)
    {
        Rectangle t = Toolbar;
        using (GraphicsPath p = Rounded(t, 8))
        {
            using (SolidBrush b = new SolidBrush(Color.FromArgb(248, 32, 32, 34)))
                g.FillPath(b, p);
            using (Pen edge = new Pen(Color.FromArgb(40, 255, 255, 255), 1))
                g.DrawPath(edge, p);
        }
        // Rectangle mode, shown as selected.
        using (GraphicsPath p = Rounded(RectButton, 5))
        using (SolidBrush b = new SolidBrush(Color.FromArgb(40, 255, 255, 255)))
            g.FillPath(b, p);
        Rectangle r = RectButton;
        using (Pen icon = new Pen(Color.White, 1.5f))
            g.DrawRectangle(icon, r.X + 9, r.Y + 11, r.Width - 18, r.Height - 22);
        // Divider.
        using (SolidBrush b = new SolidBrush(Color.FromArgb(45, 255, 255, 255)))
            g.FillRectangle(b, CloseButton.X - 8, t.Y + 12, 1, 20);
        // Close.
        Rectangle c = CloseButton;
        if (hoverClose)
        {
            using (GraphicsPath p = Rounded(c, 5))
            using (SolidBrush b = new SolidBrush(Color.FromArgb(40, 255, 255, 255)))
                g.FillPath(b, p);
        }
        using (Pen x = new Pen(Color.White, 1.5f))
        {
            g.DrawLine(x, c.X + 11, c.Y + 11, c.Right - 11, c.Bottom - 11);
            g.DrawLine(x, c.Right - 11, c.Y + 11, c.X + 11, c.Bottom - 11);
        }
    }

    protected override void OnMouseDown(MouseEventArgs e)
    {
        base.OnMouseDown(e);
        if (e.Button == MouseButtons.Right)
        {
            // A right-click throws away the box you're drawing, or cancels if there isn't one.
            if (!dragging) { Finish(null); return; }
            dragging = false;
            Capture = false;
            Invalidate();
            return;
        }
        if (e.Button != MouseButtons.Left) return;
        if (CloseButton.Contains(e.Location)) { Finish(null); return; }
        if (Toolbar.Contains(e.Location)) return;
        start = e.Location;
        current = e.Location;
        dragging = true;
        Capture = true;
        Invalidate();
    }

    protected override void OnMouseMove(MouseEventArgs e)
    {
        base.OnMouseMove(e);
        bool over = CloseButton.Contains(e.Location);
        Cursor = (Toolbar.Contains(e.Location) && !dragging) ? Cursors.Default : Cursors.Cross;
        if (over != hoverClose) { hoverClose = over; Invalidate(Toolbar); }
        if (!dragging) return;
        int x = Math.Max(0, Math.Min(e.X, ClientSize.Width));
        int y = Math.Max(0, Math.Min(e.Y, ClientSize.Height));
        current = new Point(x, y);
        Invalidate();
    }

    protected override void OnMouseUp(MouseEventArgs e)
    {
        base.OnMouseUp(e);
        if (e.Button != MouseButtons.Left || !dragging) return;
        Rectangle s = Selection;
        dragging = false;
        Capture = false;
        if (s.Width >= 4 && s.Height >= 4)
        {
            double cw = Math.Max(1, ClientSize.Width), ch = Math.Max(1, ClientSize.Height);
            CultureInfo inv = CultureInfo.InvariantCulture;
            Finish("OK " +
                (s.Left / cw).ToString("0.000000", inv) + " " +
                (s.Top / ch).ToString("0.000000", inv) + " " +
                (s.Right / cw).ToString("0.000000", inv) + " " +
                (s.Bottom / ch).ToString("0.000000", inv));
            return;
        }
        Invalidate();
    }

    protected override void OnKeyDown(KeyEventArgs e)
    {
        base.OnKeyDown(e);
        if (e.KeyCode == Keys.Escape) Finish(null);
    }

    private void Finish(string result)
    {
        watch.Stop();
        Result = result;
        // Let MacroFlow come back to the front afterwards.
        MacroFlowSnipNative.AllowSetForegroundWindow(-1);
        Close();
    }
}
'@

try {
    Add-Type -AssemblyName System.Windows.Forms, System.Drawing

    # Compiling the C# takes a few seconds, so keep the compiled copy and reuse it.
    # (MacroFlow also runs this script with -Warmup when it starts.)
    $version = 'v2'
    $dll = Join-Path $env:TEMP "macroflow-select-$version.dll"
    if (-not ('MacroFlowSnipForm' -as [type])) {
        if (Test-Path -LiteralPath $dll) {
            try { Add-Type -LiteralPath $dll } catch { Remove-Item -LiteralPath $dll -ErrorAction SilentlyContinue }
        }
        if (-not ('MacroFlowSnipForm' -as [type])) {
            try {
                Add-Type -TypeDefinition $source -ReferencedAssemblies System.Windows.Forms, System.Drawing -OutputAssembly $dll -OutputType Library
                Add-Type -LiteralPath $dll
            } catch {
                Add-Type -TypeDefinition $source -ReferencedAssemblies System.Windows.Forms, System.Drawing
            }
        }
    }
    if ($Warmup) { exit 0 }
    if (-not $ImagePath) { throw 'No screenshot was given.' }

    [MacroFlowSnipNative]::MakeDpiAware()
    [System.Windows.Forms.Application]::EnableVisualStyles()

    # Cover the same display the screenshot came from.
    $screen = $null
    if ($Device -and $Device -ne '-') {
        foreach ($s in [System.Windows.Forms.Screen]::AllScreens) {
            if ($s.DeviceName -eq $Device) { $screen = $s }
        }
    }
    if ($screen) {
        $bounds = $screen.Bounds
    } elseif ($W -gt 0 -and $H -gt 0) {
        $bounds = New-Object System.Drawing.Rectangle($X, $Y, $W, $H)
    } else {
        $bounds = [System.Windows.Forms.Screen]::FromPoint([System.Windows.Forms.Cursor]::Position).Bounds
    }

    $bitmap = [System.Drawing.Bitmap]::FromFile((Resolve-Path -LiteralPath $ImagePath).Path)
    $form = New-Object MacroFlowSnipForm -ArgumentList $bitmap, $bounds, $Hint

    # Never leave an invisible overlay behind.
    $timer = New-Object System.Windows.Forms.Timer
    $timer.Interval = 170000
    $timer.add_Tick({ $form.Close() })
    $timer.Start()

    [System.Windows.Forms.Application]::Run($form)
    $timer.Stop()
    $bitmap.Dispose()

    if ($form.Result) {
        [Console]::Out.WriteLine($form.Result)
        exit 0
    }
    exit 3
} catch {
    $message = $_.Exception.Message
    try { Add-Content -LiteralPath $logPath -Value ("{0}  {1}" -f (Get-Date -Format s), $_) } catch { }
    [Console]::Error.WriteLine($message)
    exit 2
}
