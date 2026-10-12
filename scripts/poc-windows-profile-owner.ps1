# Read-only profile check for the isolated native Desktop POC.
param(
  [Parameter(Mandatory = $true)][uint32]$DesktopProcessId,
  [Parameter(Mandatory = $true)][string]$SelectedHome
)

$ErrorActionPreference = "Stop"
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using FILETIME = System.Runtime.InteropServices.ComTypes.FILETIME;

public static class PocProfileOwner {
  [StructLayout(LayoutKind.Sequential)]
  public struct ProcessIdentity { public uint Pid; public FILETIME Started; }
  [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
  public struct ProcessInfo {
    public ProcessIdentity Process;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 256)] public string AppName;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 64)] public string ServiceName;
    public uint AppType;
    public uint AppStatus;
    public uint SessionId;
    [MarshalAs(UnmanagedType.Bool)] public bool Restartable;
  }
  [DllImport("rstrtmgr.dll", CharSet = CharSet.Unicode)]
  static extern int RmStartSession(out uint session, uint flags, System.Text.StringBuilder key);
  [DllImport("rstrtmgr.dll", CharSet = CharSet.Unicode)]
  static extern int RmRegisterResources(uint session, uint fileCount, string[] files, uint appCount, IntPtr apps, uint serviceCount, IntPtr services);
  [DllImport("rstrtmgr.dll")]
  static extern int RmGetList(uint session, out uint needed, ref uint count, [In, Out] ProcessInfo[] owners, ref uint reasons);
  [DllImport("rstrtmgr.dll")]
  static extern int RmEndSession(uint session);

  public static ProcessInfo[] Owners(string[] files) {
    uint session;
    if (RmStartSession(out session, 0, new System.Text.StringBuilder(33)) != 0) throw new Exception("Profile owner session failed");
    try {
      if (RmRegisterResources(session, (uint)files.Length, files, 0, IntPtr.Zero, 0, IntPtr.Zero) != 0) throw new Exception("Profile owner registration failed");
      uint count = 128, needed, reasons = 0;
      var owners = new ProcessInfo[count];
      if (RmGetList(session, out needed, ref count, owners, ref reasons) != 0) throw new Exception("Profile owner query failed");
      Array.Resize(ref owners, (int)count);
      return owners;
    } finally { RmEndSession(session); }
  }

  public static long Started(ProcessIdentity process) {
    return ((long)process.Started.dwHighDateTime << 32) | (uint)process.Started.dwLowDateTime;
  }
}
'@

$databases = @(Get-ChildItem -LiteralPath $SelectedHome -File |
  Where-Object { $_.Name -match '^state_[0-9]+\.sqlite$' })
if ($databases.Count -eq 0 -or $databases.Count -gt 16) { throw "Invalid fixture database count" }
if ($databases | Where-Object { $_.Attributes -band [IO.FileAttributes]::ReparsePoint }) { throw "Fixture database is a reparse point" }
$matched = @()
foreach ($owner in [PocProfileOwner]::Owners([string[]]$databases.FullName)) {
  $child = Get-CimInstance Win32_Process -Filter "ProcessId=$($owner.Process.Pid)"
  if (!$child -or $child.Name -ne "codex.exe" -or $child.ParentProcessId -ne $DesktopProcessId) { continue }
  $live = Get-Process -Id $child.ProcessId -ErrorAction SilentlyContinue
  if ($live -and $live.StartTime.ToFileTime() -eq [PocProfileOwner]::Started($owner.Process)) {
    $matched += $child.ProcessId
  }
}
if ($matched.Count -eq 0) { throw "No live child of the isolated Desktop owns the selected profile database" }
$matched
