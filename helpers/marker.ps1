# MacroFlow click marker for Windows.
# A small green dot that shows where a running macro is about to click. Clicks pass
# straight through it, it never takes focus, and it's excluded from screen capture
# (WDA_EXCLUDEFROMCAPTURE, Windows 10 2004 and later), so MacroFlow's own
# screenshots don't see it.
#
# Reads one command per line on stdin (screen pixels, the same as the mouse):
#   show <x> <y> click|move     pulse     hide
# Prints "ready" once it can take commands. Quits when stdin closes.
#   powershell -File marker.ps1 -Warmup     (just compile and cache the window code)
# MacroFlow embeds this script and runs it automatically.
# (Keep this file plain ASCII: Windows PowerShell 5.1 reads scripts as ANSI.)

param([switch]$Warmup)

$ErrorActionPreference = 'Stop'
$logPath = Join-Path $env:TEMP 'macroflow-marker.log'

# C# 5 only (Windows PowerShell compiles with the old compiler).
$source = @'
using System;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Runtime.InteropServices;
using System.Threading;
using System.Windows.Forms;

public class MacroFlowMarker : Form
{
    [DllImport("user32.dll")] static extern bool SetProcessDpiAwarenessContext(IntPtr value);
    [DllImport("user32.dll")] static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] static extern bool SetWindowDisplayAffinity(IntPtr hwnd, uint affinity);
    [DllImport("user32.dll")] static extern bool SetWindowPos(IntPtr hwnd, IntPtr after, int x, int y, int cx, int cy, uint flags);

    private const int WS_EX_TOPMOST = 0x8;
    private const int WS_EX_TRANSPARENT = 0x20;
    private const int WS_EX_TOOLWINDOW = 0x80;
    private const int WS_EX_LAYERED = 0x80000;
    private const int WS_EX_NOACTIVATE = 0x08000000;
    private const int WM_NCHITTEST = 0x84;
    private const int WM_MOUSEACTIVATE = 0x21;
    private const uint WDA_EXCLUDEFROMCAPTURE = 0x11;
    private const uint SWP_NOSIZE = 0x1, SWP_NOMOVE = 0x2, SWP_NOACTIVATE = 0x10, SWP_SHOWWINDOW = 0x40;

    // Everything this colour is see-through.
    private static readonly Color Key = Color.FromArgb(1, 1, 1);
    private static readonly Color Green = Color.FromArgb(52, 199, 123);

    private readonly System.Windows.Forms.Timer anim = new System.Windows.Forms.Timer();
    private readonly System.Windows.Forms.Timer life = new System.Windows.Forms.Timer();
    private readonly float s;
    private readonly int box;
    private float ripple = -1;
    private bool hollow;

    public static void MakeDpiAware()
    {
        try { if (SetProcessDpiAwarenessContext(new IntPtr(-4))) return; } catch (Exception) { }
        try { SetProcessDPIAware(); } catch (Exception) { }
    }

    public MacroFlowMarker()
    {
        FormBorderStyle = FormBorderStyle.None;
        StartPosition = FormStartPosition.Manual;
        ShowInTaskbar = false;
        TopMost = true;
        BackColor = Key;
        TransparencyKey = Key;
        SetStyle(ControlStyles.AllPaintingInWmPaint | ControlStyles.UserPaint |
                 ControlStyles.OptimizedDoubleBuffer, true);
        using (Graphics g = Graphics.FromHwnd(IntPtr.Zero)) { s = g.DpiX / 96f; }
        box = (int)Math.Round(60 * s);
        Size = new Size(box, box);
        Location = new Point(-box * 4, -box * 4);
        anim.Interval = 16;
        anim.Tick += OnAnim;
        life.Interval = 3500;
        life.Tick += delegate { HideDot(); };
    }

    protected override bool ShowWithoutActivation { get { return true; } }

    protected override CreateParams CreateParams
    {
        get
        {
            CreateParams cp = base.CreateParams;
            cp.ExStyle |= WS_EX_TOPMOST | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_NOACTIVATE;
            return cp;
        }
    }

    protected override void WndProc(ref Message m)
    {
        if (m.Msg == WM_NCHITTEST) { m.Result = new IntPtr(-1); return; }      // HTTRANSPARENT
        if (m.Msg == WM_MOUSEACTIVATE) { m.Result = new IntPtr(3); return; }   // MA_NOACTIVATE
        base.WndProc(ref m);
    }

    protected override void OnHandleCreated(EventArgs e)
    {
        base.OnHandleCreated(e);
        // Leave this window out of screenshots (Windows 10 2004+). If that isn't
        // supported, MacroFlow still hides the dot before every screenshot.
        try { SetWindowDisplayAffinity(Handle, WDA_EXCLUDEFROMCAPTURE); } catch (Exception) { }
    }

    public void ShowDot(int x, int y, bool ring)
    {
        anim.Stop();
        ripple = -1;
        hollow = ring;
        Location = new Point(x - box / 2, y - box / 2);
        if (!Visible) Show();
        SetWindowPos(Handle, new IntPtr(-1), 0, 0, 0, 0, SWP_NOSIZE | SWP_NOMOVE | SWP_NOACTIVATE | SWP_SHOWWINDOW);
        Invalidate();
        life.Stop();
        life.Start();
    }

    public void Pulse()
    {
        if (!Visible) return;
        life.Stop();
        ripple = 0;
        anim.Start();
    }

    public void HideDot()
    {
        anim.Stop();
        life.Stop();
        Hide();
    }

    private void OnAnim(object sender, EventArgs e)
    {
        ripple += 1f / 18f; // about 300 ms
        if (ripple >= 1) { HideDot(); return; }
        Invalidate();
    }

    private void Circle(Graphics g, Pen pen, Brush brush, float r)
    {
        float c = box / 2f;
        if (brush != null) g.FillEllipse(brush, c - r, c - r, r * 2, r * 2);
        if (pen != null) g.DrawEllipse(pen, c - r, c - r, r * 2, r * 2);
    }

    protected override void OnPaint(PaintEventArgs e)
    {
        Graphics g = e.Graphics;
        g.Clear(Key);
        g.SmoothingMode = SmoothingMode.AntiAlias;
        if (ripple >= 0)
        {
            using (Pen p = new Pen(Green, (3 * (1 - ripple) + 1) * s))
                Circle(g, p, null, (8 + ripple * 18) * s);
        }
        // The dot shrinks away during the ripple.
        float r = 7 * s * (ripple >= 0 ? Math.Max(0, 1 - ripple * 1.6f) : 1);
        if (r < 0.5f) return;
        if (hollow)
        {
            using (Pen p = new Pen(Green, 2.5f * s)) Circle(g, p, null, r);
        }
        else
        {
            using (SolidBrush b = new SolidBrush(Green))
            using (Pen p = new Pen(Color.White, 1.5f * s))
                Circle(g, p, b, r);
        }
    }

    /// Read commands on a background thread and carry them out on the window's thread.
    public void StartReading()
    {
        Thread t = new Thread(delegate()
        {
            try
            {
                string line;
                while ((line = Console.In.ReadLine()) != null)
                {
                    string l = line;
                    BeginInvoke((MethodInvoker)delegate { Run(l); });
                }
            }
            catch (Exception) { }
            try { BeginInvoke((MethodInvoker)delegate { Application.Exit(); }); } catch (Exception) { }
        });
        t.IsBackground = true;
        t.Start();
    }

    private void Run(string line)
    {
        string[] p = line.Trim().Split(' ');
        if (p.Length == 0) return;
        int x, y;
        if (p[0] == "show" && p.Length >= 3 && int.TryParse(p[1], out x) && int.TryParse(p[2], out y))
            ShowDot(x, y, p.Length > 3 && p[3] == "move");
        else if (p[0] == "pulse") Pulse();
        else if (p[0] == "hide") HideDot();
    }
}
'@

try {
    Add-Type -AssemblyName System.Windows.Forms, System.Drawing

    # Compiling the C# takes a few seconds, so keep the compiled copy and reuse it.
    $version = 'v1'
    $dll = Join-Path $env:TEMP "macroflow-marker-$version.dll"
    if (-not ('MacroFlowMarker' -as [type])) {
        if (Test-Path -LiteralPath $dll) {
            try { Add-Type -LiteralPath $dll } catch { Remove-Item -LiteralPath $dll -ErrorAction SilentlyContinue }
        }
        if (-not ('MacroFlowMarker' -as [type])) {
            try {
                Add-Type -TypeDefinition $source -ReferencedAssemblies System.Windows.Forms, System.Drawing -OutputAssembly $dll -OutputType Library
                Add-Type -LiteralPath $dll
            } catch {
                Add-Type -TypeDefinition $source -ReferencedAssemblies System.Windows.Forms, System.Drawing
            }
        }
    }
    if ($Warmup) { exit 0 }

    [MacroFlowMarker]::MakeDpiAware()
    $form = New-Object MacroFlowMarker
    $null = $form.Handle   # create the window (hidden) so commands can reach it
    $form.StartReading()
    [Console]::Out.WriteLine('ready')
    [Console]::Out.Flush()
    [System.Windows.Forms.Application]::Run()
    exit 0
} catch {
    try { Add-Content -LiteralPath $logPath -Value ("{0}  {1}" -f (Get-Date -Format s), $_) } catch { }
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 2
}
