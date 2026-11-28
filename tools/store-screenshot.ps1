# Captures the HomeKVM window for Microsoft Store screenshots, from a dev build in its
# Store screenshot mode (sample PCs, no real names or addresses). See STORE.md.
#   cargo build --no-default-features
#   pwsh tools/store-screenshot.ps1 -Theme dark  -Out target/store-art/window-dark.png
#   pwsh tools/store-screenshot.ps1 -Theme light -Out target/store-art/window-light.png
#   pwsh tools/store-screenshot.ps1 -Theme dark  -Mode store-pair -Out target/store-art/window-pair.png
# Stop any running HomeKVM first: only one copy runs at a time.
param([string]$Theme = "dark", [string]$Out, [string]$Mode = "store")
New-Item -ItemType Directory -Force (Split-Path $Out) | Out-Null
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System; using System.Text; using System.Runtime.InteropServices;
public static class H {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [StructLayout(LayoutKind.Sequential)] public struct R { public int L, T, Rt, B; }
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc p, IntPtr l);
  [DllImport("user32.dll")] public static extern int GetWindowThreadProcessId(IntPtr h, out int pid);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out R r);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint f);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  public static IntPtr Find(int pid) {
    IntPtr found = IntPtr.Zero;
    EnumWindows((h, l) => { int p; GetWindowThreadProcessId(h, out p);
      if (p == pid) { var sb = new StringBuilder(256); GetWindowText(h, sb, 256); if (sb.ToString() == "HomeKVM") { found = h; return false; } }
      return true; }, IntPtr.Zero);
    return found;
  }
}
"@
[H]::SetProcessDPIAware() | Out-Null
$exe = Join-Path $PSScriptRoot "..\target\debug\HomeKVM.exe"
$env:HOMEKVM_DEMO = $Mode; $env:HOMEKVM_THEME = $Theme
$p = Start-Process -FilePath $exe -PassThru
Start-Sleep -Seconds 6
$h = [H]::Find($p.Id)
Start-Process -FilePath $exe -Wait | Out-Null
$wait = [Diagnostics.Stopwatch]::StartNew()
while (-not [H]::IsWindowVisible($h) -and $wait.ElapsedMilliseconds -lt 8000) { Start-Sleep -Milliseconds 20 }
Start-Sleep -Seconds 3
$r = New-Object H+R; [H]::GetWindowRect($h, [ref]$r) | Out-Null
$bmp = New-Object System.Drawing.Bitmap ($r.Rt - $r.L), ($r.B - $r.T)
$g = [System.Drawing.Graphics]::FromImage($bmp); $hdc = $g.GetHdc(); [H]::PrintWindow($h, $hdc, 2) | Out-Null; $g.ReleaseHdc($hdc)
$bmp.Save($Out); $g.Dispose(); $bmp.Dispose()
"saved $Out ($($r.Rt - $r.L) x $($r.B - $r.T))"
Stop-Process -Id $p.Id
