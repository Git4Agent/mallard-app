[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$BundleDirectory,

    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$TemporaryRoot,

    [Parameter(Mandatory = $false)]
    [string]$GitHubOutputPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
if ($PSVersionTable.PSEdition -ne "Core" -or -not $IsWindows) {
    throw "Windows bundle verification requires PowerShell 7 on Windows."
}

function Get-ExistingDirectoryPath {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,

        [Parameter(Mandatory = $true)]
        [string]$Label
    )

    $item = Get-Item -LiteralPath $Path -ErrorAction Stop
    if (-not $item.PSIsContainer) {
        throw "$Label is not a directory."
    }
    $separators = [char[]]@(
        [System.IO.Path]::DirectorySeparatorChar,
        [System.IO.Path]::AltDirectorySeparatorChar
    )
    return [System.IO.Path]::GetFullPath($item.FullName).TrimEnd($separators)
}

function Assert-DirectChildPath {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Parent,

        [Parameter(Mandatory = $true)]
        [string]$Child
    )

    $childParent = [System.IO.Path]::GetDirectoryName($Child)
    if (-not [string]::Equals($Parent, $childParent, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "The verification directory escaped its temporary parent."
    }
}

function New-ProcessStartInfo {
    param(
        [Parameter(Mandatory = $true)]
        [string]$FilePath,

        [Parameter(Mandatory = $true)]
        [string[]]$Arguments
    )

    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $FilePath
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    foreach ($argument in $Arguments) {
        $startInfo.ArgumentList.Add($argument)
    }
    return $startInfo
}

function Invoke-Installer {
    param(
        [Parameter(Mandatory = $true)]
        [string]$InstallerPath,

        [Parameter(Mandatory = $true)]
        [string]$InstallDirectory
    )

    if ($InstallDirectory.Contains('"') -or $InstallDirectory.Contains("`r") -or $InstallDirectory.Contains("`n")) {
        throw "The NSIS installation directory contains unsupported characters."
    }
    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $InstallerPath
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    # NSIS requires /D to be the final raw argument without surrounding quotes;
    # it consumes the remainder of the command line as the directory, including spaces.
    $startInfo.Arguments = "/S /NS /D=$InstallDirectory"
    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    try {
        if (-not $process.Start()) {
            throw "The NSIS installer did not start."
        }
        if (-not $process.WaitForExit(300000)) {
            $process.Kill($true)
            $process.WaitForExit()
            throw "The NSIS installer exceeded the five-minute verification timeout."
        }
        if ($process.ExitCode -ne 0) {
            throw "The NSIS installer exited with status $($process.ExitCode)."
        }
    }
    finally {
        $process.Dispose()
    }
}

function Invoke-Uninstaller {
    param(
        [Parameter(Mandatory = $true)]
        [string]$UninstallerPath,

        [Parameter(Mandatory = $true)]
        [string]$InstallDirectory
    )

    if ($InstallDirectory.Contains('"') -or $InstallDirectory.Contains("`r") -or $InstallDirectory.Contains("`n")) {
        throw "The NSIS uninstall directory contains unsupported characters."
    }
    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $UninstallerPath
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    # _?= keeps the copied uninstaller synchronous and must be the final raw argument.
    $startInfo.Arguments = "/S _?=$InstallDirectory"
    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    try {
        if (-not $process.Start()) {
            throw "The NSIS uninstaller did not start."
        }
        if (-not $process.WaitForExit(120000)) {
            $process.Kill($true)
            $process.WaitForExit()
            throw "The NSIS uninstaller exceeded the two-minute cleanup timeout."
        }
        if ($process.ExitCode -ne 0) {
            throw "The NSIS uninstaller exited with status $($process.ExitCode)."
        }
    }
    finally {
        $process.Dispose()
    }
}

function Get-CliHelp {
    param(
        [Parameter(Mandatory = $true)]
        [string]$CliPath
    )

    $startInfo = New-ProcessStartInfo -FilePath $CliPath -Arguments @("--help")
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    try {
        if (-not $process.Start()) {
            throw "The installed Mallard CLI did not start."
        }
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit(30000)) {
            $process.Kill($true)
            $process.WaitForExit()
            throw "The installed Mallard CLI exceeded the help timeout."
        }
        $stdout = $stdoutTask.GetAwaiter().GetResult()
        $stderr = $stderrTask.GetAwaiter().GetResult()
        if ($process.ExitCode -ne 0) {
            throw "The installed Mallard CLI help command exited with status $($process.ExitCode)."
        }
        $help = "$stdout`n$stderr"
        if ($help.Length -gt 65536) {
            throw "The installed Mallard CLI returned unexpectedly large help output."
        }
        return $help
    }
    finally {
        $process.Dispose()
    }
}

$bundleRoot = Get-ExistingDirectoryPath -Path $BundleDirectory -Label "NSIS bundle directory"
$temporaryRootPath = Get-ExistingDirectoryPath -Path $TemporaryRoot -Label "Temporary root"
$separators = [char[]]@(
    [System.IO.Path]::DirectorySeparatorChar,
    [System.IO.Path]::AltDirectorySeparatorChar
)
$filesystemRoot = [System.IO.Path]::GetPathRoot($temporaryRootPath).TrimEnd($separators)
if ([string]::Equals($temporaryRootPath, $filesystemRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "The filesystem root cannot be used as the verification temporary root."
}

$installers = @(Get-ChildItem -LiteralPath $bundleRoot -Filter "*.exe" -File)
if ($installers.Count -ne 1) {
    throw "Expected exactly one generated NSIS installer, found $($installers.Count)."
}
$installerPath = [System.IO.Path]::GetFullPath($installers[0].FullName)
if ($installers[0].Length -le 0) {
    throw "The generated NSIS installer is empty."
}

$verificationRoot = [System.IO.Path]::GetFullPath(
    (Join-Path $temporaryRootPath "mallard-nsis-verification-$([guid]::NewGuid().ToString('N'))")
)
Assert-DirectChildPath -Parent $temporaryRootPath -Child $verificationRoot
$installDirectory = Join-Path $verificationRoot "installed"
Assert-DirectChildPath -Parent $verificationRoot -Child $installDirectory
$uninstallRegistryPath = "Registry::HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Uninstall\Mallard"
if (Test-Path -LiteralPath $uninstallRegistryPath) {
    throw "Refusing to replace a pre-existing current-user Mallard installation."
}

New-Item -ItemType Directory -Path $installDirectory -ErrorAction Stop | Out-Null
$pathBeforeInstall = [pscustomobject]@{
    Process = $env:Path
    User = [System.Environment]::GetEnvironmentVariable("Path", "User")
    Machine = [System.Environment]::GetEnvironmentVariable("Path", "Machine")
}
$installerSucceeded = $false
try {
    Invoke-Installer -InstallerPath $installerPath -InstallDirectory $installDirectory
    $installerSucceeded = $true

    $uninstallMetadata = Get-ItemProperty -LiteralPath $uninstallRegistryPath -ErrorAction Stop
    $registeredInstallDirectory = [System.IO.Path]::GetFullPath(
        ([string]$uninstallMetadata.InstallLocation).Trim([char]'"')
    ).TrimEnd($separators)
    if (-not [string]::Equals(
        $installDirectory,
        $registeredInstallDirectory,
        [System.StringComparison]::OrdinalIgnoreCase
    )) {
        throw "The NSIS installer registered an unexpected installation directory."
    }

    $pathAfterInstall = [pscustomobject]@{
        Process = $env:Path
        User = [System.Environment]::GetEnvironmentVariable("Path", "User")
        Machine = [System.Environment]::GetEnvironmentVariable("Path", "Machine")
    }
    foreach ($scope in @("Process", "User", "Machine")) {
        if (-not [string]::Equals(
            $pathBeforeInstall.$scope,
            $pathAfterInstall.$scope,
            [System.StringComparison]::Ordinal
        )) {
            throw "The NSIS installer unexpectedly changed the $scope PATH."
        }
    }

    $installedCli = Join-Path $installDirectory "mallard-cli.exe"
    $cliItem = Get-Item -LiteralPath $installedCli -ErrorAction Stop
    if ($cliItem.PSIsContainer -or $cliItem.Length -le 0) {
        throw "The NSIS installer did not install a valid mallard-cli.exe."
    }

    $help = Get-CliHelp -CliPath $cliItem.FullName
    if ($help -notmatch "(?im)^Usage:\s+mallard-cli\b" -or $help -notmatch "(?im)^\s*storage\s") {
        throw "The installed Mallard CLI did not return the expected help output."
    }

    if ($GitHubOutputPath) {
        if ($installerPath.Contains("`r") -or $installerPath.Contains("`n")) {
            throw "The installer path cannot be written to GitHub Actions output."
        }
        Add-Content -LiteralPath $GitHubOutputPath -Value "installer_path=$installerPath" -Encoding utf8
    }
    Write-Output $installerPath
}
finally {
    $cleanupFailure = $null
    try {
        $installedUninstaller = Join-Path $installDirectory "uninstall.exe"
        if (Test-Path -LiteralPath $installedUninstaller -PathType Leaf) {
            $copiedUninstaller = Join-Path $verificationRoot "mallard-uninstall.exe"
            Assert-DirectChildPath -Parent $verificationRoot -Child $copiedUninstaller
            Assert-DirectChildPath -Parent $verificationRoot -Child $installDirectory
            Copy-Item -LiteralPath $installedUninstaller -Destination $copiedUninstaller -ErrorAction Stop
            Invoke-Uninstaller -UninstallerPath $copiedUninstaller -InstallDirectory $installDirectory

            if (Test-Path -LiteralPath (Join-Path $installDirectory "mallard-cli.exe")) {
                throw "The NSIS uninstaller left mallard-cli.exe installed."
            }
            if (Test-Path -LiteralPath $installDirectory) {
                throw "The NSIS uninstaller left the temporary installation directory behind."
            }
        }
        elseif ($installerSucceeded) {
            throw "The successful NSIS installation did not provide uninstall.exe."
        }

        if (Test-Path -LiteralPath $uninstallRegistryPath) {
            throw "The NSIS uninstaller left current-user uninstall metadata behind."
        }
        $pathAfterCleanup = [pscustomobject]@{
            Process = $env:Path
            User = [System.Environment]::GetEnvironmentVariable("Path", "User")
            Machine = [System.Environment]::GetEnvironmentVariable("Path", "Machine")
        }
        foreach ($scope in @("Process", "User", "Machine")) {
            if (-not [string]::Equals(
                $pathBeforeInstall.$scope,
                $pathAfterCleanup.$scope,
                [System.StringComparison]::Ordinal
            )) {
                throw "Windows bundle verification changed the $scope PATH."
            }
        }
    }
    catch {
        $cleanupFailure = $_
    }

    try {
        Assert-DirectChildPath -Parent $temporaryRootPath -Child $verificationRoot
        if (Test-Path -LiteralPath $verificationRoot) {
            Remove-Item -LiteralPath $verificationRoot -Recurse -Force
        }
        if (Test-Path -LiteralPath $verificationRoot) {
            throw "The verification temporary directory could not be removed."
        }
    }
    catch {
        if ($null -eq $cleanupFailure) {
            $cleanupFailure = $_
        }
    }

    if ($null -ne $cleanupFailure) {
        throw $cleanupFailure
    }
}
