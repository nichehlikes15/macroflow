# MacroFlow pop-up notification for Windows.
# Shows a small card at the top of the screen without taking focus away from the
# app the macro is working in. Click it (or wait) to dismiss.
#
#   powershell -File notify.ps1 -Title b64:... -Message b64:... -Style info -Position top-center -DurationMs 3000 [-Wait] [-Sound]
#   powershell -File notify.ps1 -Warmup      (just compile and cache the window code)
#
# Title and message are passed as "b64:" + base64 of the UTF-8 text, so any characters
# (and empty text) survive the trip through the command line.
# Style: info | success | warning | error.   Position: top-center | top-right.
# With -Wait it stays until clicked. Exits 0 when it closes, 2 on error (see the log).
# MacroFlow embeds this script and runs it automatically.
# (Keep this file plain ASCII: Windows PowerShell 5.1 reads scripts as ANSI.)

param(
    [string]$Title = '',
    [string]$Message = '',
    [string]$Style = 'info',
    [string]$Position = 'top-center',
    [int]$DurationMs = 3000,
    [switch]$Wait,
    [switch]$Sound,
    [switch]$Warmup
)

$ErrorActionPreference = 'Stop'
$logPath = Join-Path $env:TEMP 'macroflow-notify.log'

function Decode-Text([string]$s) {
    if ($s -and $s.StartsWith('b64:')) {
        $data = $s.Substring(4)
        if (-not $data) { return '' }
        return [System.Text.Encoding]::UTF8.GetString([System.Convert]::FromBase64String($data))
    }
    return $s
}

# C# 5 only (Windows PowerShell compiles with the old compiler): no $"", ?., => members.
$source = @'
using System;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Drawing.Text;
using System.Runtime.InteropServices;
using System.Windows.Forms;

public static class MacroFlowToastNative
{
    [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr value);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("dwmapi.dll")] public static extern int DwmSetWindowAttribute(IntPtr hwnd, int attr, ref int value, int size);

    public static void MakeDpiAware()
    {
        try { if (SetProcessDpiAwarenessContext(new IntPtr(-4))) return; } catch (Exception) { }
        try { SetProcessDPIAware(); } catch (Exception) { }
    }

    /// Windows 11 rounded corners. Returns false on Windows 10, where it isn't supported.
    public static bool RoundCorners(IntPtr hwnd)
    {
        try
        {
            int round = 2; // DWMWCP_ROUND
            return DwmSetWindowAttribute(hwnd, 33, ref round, 4) == 0;
        }
        catch (Exception) { return false; }
    }
}

public class MacroFlowToast : Form
{
    private const int WS_EX_TOPMOST = 0x8;
    private const int WS_EX_TOOLWINDOW = 0x80;
    private const int WS_EX_NOACTIVATE = 0x08000000;
    private const int CS_DROPSHADOW = 0x20000;
    private const int WM_MOUSEACTIVATE = 0x21;
    private const int MA_NOACTIVATE = 3;

    private readonly string title;
    private readonly string message;
    private readonly Color accent;
    private readonly string symbol;
    private readonly bool wait;
    private readonly int durationMs;
    private readonly float s;
    private readonly Font titleFont = new Font("Segoe UI Semibold", 10.5f);
    private readonly Font bodyFont = new Font("Segoe UI", 9.5f);
    private readonly Font hintFont = new Font("Segoe UI Semibold", 8.5f);
    private readonly Font symbolFont = new Font("Segoe UI", 11f, FontStyle.Bold);
    private const TextFormatFlags Flow = TextFormatFlags.WordBreak | TextFormatFlags.NoPrefix;
    private const string HintText = "Click to continue";

    private int pad, iconSize, textX, textW, titleH, bodyH, hintH, targetTop;
    private Rectangle closeBox;
    private bool hoverClose;
    private bool closing;
    private int frame;
    private readonly Timer anim = new Timer();
    private readonly Timer life = new Timer();

    public MacroFlowToast(string title, string message, string style, string position, int durationMs, bool wait)
    {
        this.title = string.IsNullOrEmpty(title) ? "MacroFlow" : title;
        this.message = message ?? "";
        this.wait = wait;
        this.durationMs = Math.Max(500, durationMs);
        switch (style)
        {
            case "success": accent = Color.FromArgb(52, 199, 123); symbol = "\u2713"; break;
            case "warning": accent = Color.FromArgb(245, 184, 74); symbol = "!"; break;
            case "error": accent = Color.FromArgb(239, 91, 91); symbol = "\u00D7"; break;
            default: accent = Color.FromArgb(124, 92, 255); symbol = "i"; break;
        }

        FormBorderStyle = FormBorderStyle.None;
        StartPosition = FormStartPosition.Manual;
        ShowInTaskbar = false;
        BackColor = Color.FromArgb(24, 26, 34);
        Text = "MacroFlow";
        Cursor = Cursors.Hand;
        SetStyle(ControlStyles.AllPaintingInWmPaint | ControlStyles.UserPaint |
                 ControlStyles.OptimizedDoubleBuffer, true);
        DoubleBuffered = true;

        using (Graphics g = Graphics.FromHwnd(IntPtr.Zero)) { s = g.DpiX / 96f; }
        Arrange(position == "top-right");
        Opacity = 0;
    }

    private int S(float v) { return (int)Math.Round(v * s); }

    private void Arrange(bool right)
    {
        int width = S(380);
        pad = S(16);
        iconSize = S(32);
        textX = pad + iconSize + S(12);
        textW = width - textX - pad - S(18);
        Size limit = new Size(textW, 100000);
        titleH = TextRenderer.MeasureText(title, titleFont, limit, Flow).Height;
        bodyH = message.Length > 0 ? TextRenderer.MeasureText(message, bodyFont, limit, Flow).Height : 0;
        hintH = wait ? TextRenderer.MeasureText(HintText, hintFont, limit, Flow).Height : 0;
        int height = pad + titleH + (bodyH > 0 ? S(4) + bodyH : 0) + (hintH > 0 ? S(6) + hintH : 0) + pad;
        height = Math.Max(height, iconSize + pad * 2);

        // The screen the mouse is on, top centre (or top right), clear of the taskbar.
        Rectangle wa = Screen.FromPoint(Cursor.Position).WorkingArea;
        int left = right ? wa.Right - width - S(16) : wa.Left + (wa.Width - width) / 2;
        targetTop = wa.Top + S(14);
        Bounds = new Rectangle(left, targetTop - S(12), width, height);
        closeBox = new Rectangle(width - pad - S(12), pad, S(12), S(12));
    }

    // Never take focus: the macro keeps typing into its own app.
    protected override bool ShowWithoutActivation { get { return true; } }

    protected override CreateParams CreateParams
    {
        get
        {
            CreateParams cp = base.CreateParams;
            cp.ExStyle |= WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
            cp.ClassStyle |= CS_DROPSHADOW;
            return cp;
        }
    }

    protected override void WndProc(ref Message m)
    {
        if (m.Msg == WM_MOUSEACTIVATE)
        {
            m.Result = new IntPtr(MA_NOACTIVATE);
            return;
        }
        base.WndProc(ref m);
    }

    protected override void OnHandleCreated(EventArgs e)
    {
        base.OnHandleCreated(e);
        if (!MacroFlowToastNative.RoundCorners(Handle))
        {
            using (GraphicsPath p = Rounded(new Rectangle(0, 0, Width, Height), S(10)))
                Region = new Region(p);
        }
    }

    protected override void OnShown(EventArgs e)
    {
        base.OnShown(e);
        // Slide down and fade in.
        anim.Interval = 15;
        anim.Tick += delegate
        {
            frame++;
            if (closing)
            {
                Opacity = Math.Max(0, Opacity - 0.12);
                if (Opacity <= 0.01) { anim.Stop(); Close(); }
                return;
            }
            double t = Math.Min(1.0, frame / 12.0);
            Opacity = t;
            Top = targetTop - (int)Math.Round((1 - t) * S(12));
            if (t >= 1.0)
            {
                anim.Stop();
                if (!wait) life.Start();
            }
        };
        life.Interval = durationMs;
        life.Tick += delegate { BeginClose(); };
        anim.Start();
    }

    private void BeginClose()
    {
        if (closing) return;
        closing = true;
        life.Stop();
        anim.Start();
    }

    protected override void OnMouseDown(MouseEventArgs e)
    {
        base.OnMouseDown(e);
        BeginClose();
    }

    protected override void OnMouseMove(MouseEventArgs e)
    {
        base.OnMouseMove(e);
        Rectangle hit = closeBox;
        hit.Inflate(S(6), S(6));
        bool over = hit.Contains(e.Location);
        if (over != hoverClose) { hoverClose = over; Invalidate(); }
    }

    protected override void OnMouseLeave(EventArgs e)
    {
        base.OnMouseLeave(e);
        if (hoverClose) { hoverClose = false; Invalidate(); }
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
        g.SmoothingMode = SmoothingMode.AntiAlias;
        g.TextRenderingHint = TextRenderingHint.ClearTypeGridFit;

        // Coloured stripe down the left edge, and a faint border.
        using (SolidBrush b = new SolidBrush(accent))
            g.FillRectangle(b, 0, 0, S(4), Height);
        using (Pen border = new Pen(Color.FromArgb(55, 60, 75), 1))
        {
            g.SmoothingMode = SmoothingMode.None;
            g.DrawRectangle(border, 0, 0, Width - 1, Height - 1);
            g.SmoothingMode = SmoothingMode.AntiAlias;
        }

        // Icon.
        Rectangle icon = new Rectangle(pad, pad, iconSize, iconSize);
        using (SolidBrush b = new SolidBrush(Color.FromArgb(46, accent)))
            g.FillEllipse(b, icon);
        TextRenderer.DrawText(g, symbol, symbolFont, icon, accent,
            TextFormatFlags.HorizontalCenter | TextFormatFlags.VerticalCenter | TextFormatFlags.NoPrefix);

        // Text.
        int y = pad;
        TextRenderer.DrawText(g, title, titleFont, new Rectangle(textX, y, textW, titleH),
            Color.FromArgb(232, 234, 240), Flow);
        y += titleH;
        if (bodyH > 0)
        {
            y += S(4);
            TextRenderer.DrawText(g, message, bodyFont, new Rectangle(textX, y, textW, bodyH),
                Color.FromArgb(172, 178, 192), Flow);
            y += bodyH;
        }
        if (hintH > 0)
        {
            y += S(6);
            TextRenderer.DrawText(g, HintText, hintFont, new Rectangle(textX, y, textW, hintH), accent, Flow);
        }

        // Close cross.
        Rectangle c = closeBox;
        c.Inflate(-S(2), -S(2));
        using (Pen x = new Pen(hoverClose ? Color.FromArgb(235, 235, 235) : Color.FromArgb(120, 126, 140), 1.5f * s))
        {
            g.DrawLine(x, c.Left, c.Top, c.Right, c.Bottom);
            g.DrawLine(x, c.Right, c.Top, c.Left, c.Bottom);
        }
    }
}
'@

try {
    Add-Type -AssemblyName System.Windows.Forms, System.Drawing

    # Compiling the C# takes a few seconds, so keep the compiled copy and reuse it.
    # (MacroFlow also runs this script with -Warmup when it starts.)
    $version = 'v1'
    $dll = Join-Path $env:TEMP "macroflow-notify-$version.dll"
    if (-not ('MacroFlowToast' -as [type])) {
        if (Test-Path -LiteralPath $dll) {
            try { Add-Type -LiteralPath $dll } catch { Remove-Item -LiteralPath $dll -ErrorAction SilentlyContinue }
        }
        if (-not ('MacroFlowToast' -as [type])) {
            try {
                Add-Type -TypeDefinition $source -ReferencedAssemblies System.Windows.Forms, System.Drawing -OutputAssembly $dll -OutputType Library
                Add-Type -LiteralPath $dll
            } catch {
                Add-Type -TypeDefinition $source -ReferencedAssemblies System.Windows.Forms, System.Drawing
            }
        }
    }
    if ($Warmup) { exit 0 }

    [MacroFlowToastNative]::MakeDpiAware()
    [System.Windows.Forms.Application]::EnableVisualStyles()

    $titleText = Decode-Text $Title
    $messageText = Decode-Text $Message

    if ($Sound) {
        switch ($Style) {
            'error' { [System.Media.SystemSounds]::Hand.Play() }
            'warning' { [System.Media.SystemSounds]::Exclamation.Play() }
            default { [System.Media.SystemSounds]::Asterisk.Play() }
        }
    }

    $form = New-Object MacroFlowToast -ArgumentList $titleText, $messageText, $Style, $Position, $DurationMs, $Wait.IsPresent
    [System.Windows.Forms.Application]::Run($form)
    exit 0
} catch {
    $message = $_.Exception.Message
    try { Add-Content -LiteralPath $logPath -Value ("{0}  {1}" -f (Get-Date -Format s), $_) } catch { }
    [Console]::Error.WriteLine($message)
    exit 2
}
