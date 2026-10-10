# Tries the NSIS installer built in target/release/bundle/nsis as an update,
# clicked through as a person would, on a Windows machine where Cairn may be
# installed and changed: CI's windows-installer job runs it. It installs the
# build with /S, then makes that install look older (DisplayVersion 1.0.0 in
# its uninstall entry, which is all the installer reads of the version) and
# runs the installer three times:
#
#   1. Interactively, over the "older" install: one window from Welcome to
#      Installing to Finish. No "Already Installed" page, no folder page and
#      no window of the old version's uninstaller, which offers to delete the
#      app data. The app data, and a change to the Start menu shortcut, stay.
#   2. Interactively, over the same version: Tauri's "Already Installed" page
#      as before, then the install, with the app data kept.
#   3. With /P over the "older" install: the app data kept.
#
# The installer's window is driven by posting Next (WM_COMMAND IDOK) to it,
# which NSIS ignores while Next is disabled, so the window needs no focus.
# On the Finish page every box is unticked first, so that Cairn does not start.
#
#   pwsh scripts/windows-installer-update.ps1
$ErrorActionPreference = 'Stop'

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public static class Win {
    delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent, EnumProc f, IntPtr l);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern bool IsWindowEnabled(IntPtr h);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern IntPtr GetDlgItem(IntPtr h, int id);
    [DllImport("user32.dll")] static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr SendMessageTimeout(IntPtr h, uint msg, IntPtr w, StringBuilder l, uint flags, uint ms, out IntPtr result);
    [DllImport("user32.dll")] static extern IntPtr SendMessageTimeout(IntPtr h, uint msg, IntPtr w, IntPtr l, uint flags, uint ms, out IntPtr result);
    [DllImport("user32.dll")] static extern int GetWindowLong(IntPtr h, int index);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr FindWindowEx(IntPtr parent, IntPtr after, string cls, string title);

    const uint WM_GETTEXT = 0x000D, WM_COMMAND = 0x0111, BM_GETCHECK = 0x00F0, BM_SETCHECK = 0x00F1, SMTO_ABORTIFHUNG = 2;

    /// A window's text, also of a control in another process.
    public static string Text(IntPtr h) {
        var s = new StringBuilder(1024);
        IntPtr r;
        SendMessageTimeout(h, WM_GETTEXT, (IntPtr)s.Capacity, s, SMTO_ABORTIFHUNG, 2000, out r);
        return s.ToString().Trim();
    }

    static string ClassOf(IntPtr h) {
        var s = new StringBuilder(256);
        GetClassName(h, s, s.Capacity);
        return s.ToString();
    }

    /// The titles of the visible top-level windows.
    public static List<string> Titles() {
        var all = new List<string>();
        EnumWindows((h, l) => { if (IsWindowVisible(h)) all.Add(Text(h)); return true; }, IntPtr.Zero);
        return all;
    }

    /// The visible dialog window of process `pid`, or zero.
    public static IntPtr DialogOf(int pid) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, l) => {
            uint p;
            GetWindowThreadProcessId(h, out p);
            if (p == pid && IsWindowVisible(h) && ClassOf(h) == "#32770") { found = h; return false; }
            return true;
        }, IntPtr.Zero);
        return found;
    }

    /// The texts of the visible windows inside `parent`.
    public static List<string> Texts(IntPtr parent) {
        var all = new List<string>();
        EnumChildWindows(parent, (h, l) => { if (IsWindowVisible(h)) all.Add(Text(h)); return true; }, IntPtr.Zero);
        return all;
    }

    /// The page inside the installer's window.
    public static IntPtr Page(IntPtr dialog) {
        return FindWindowEx(dialog, IntPtr.Zero, "#32770", null);
    }

    public static bool NextEnabled(IntPtr dialog) {
        IntPtr next = GetDlgItem(dialog, 1);
        return next != IntPtr.Zero && IsWindowVisible(next) && IsWindowEnabled(next);
    }

    public static void Next(IntPtr dialog) {
        PostMessage(dialog, WM_COMMAND, (IntPtr)1, IntPtr.Zero);
    }

    /// Unticks every check box on the page; returns their texts.
    public static List<string> Untick(IntPtr page) {
        var boxes = new List<string>();
        EnumChildWindows(page, (h, l) => {
            int kind = GetWindowLong(h, -16) & 0xF;
            if (ClassOf(h) == "Button" && (kind == 2 || kind == 3)) {
                IntPtr r;
                SendMessageTimeout(h, BM_SETCHECK, IntPtr.Zero, IntPtr.Zero, SMTO_ABORTIFHUNG, 2000, out r);
                boxes.Add(Text(h));
            }
            return true;
        }, IntPtr.Zero);
        return boxes;
    }
}
'@

$setup = (Get-ChildItem target/release/bundle/nsis/*-setup.exe | Select-Object -First 1).FullName
$version = (Get-Content app/src-tauri/tauri.conf.json -Raw | ConvertFrom-Json).version
$entry = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Cairn'
$data = @("$env:APPDATA\app.cairn.notes", "$env:LOCALAPPDATA\app.cairn.notes")
$startMenu = "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\Cairn.lnk"
$failed = [System.Collections.Generic.List[string]]::new()

# The pages, by their titles, in the order they are looked for.
$titles = @('Already Installed', 'Choose Install Location', 'Installation Complete', 'Installing',
    'Welcome to Cairn Setup', 'Completing Cairn Setup')

function Fail([string]$what) {
    Write-Host "FAIL: $what"
    $failed.Add($what)
}

function Set-Version([string]$v) {
    Set-ItemProperty -Path $entry -Name DisplayVersion -Value $v
}

function Get-Version {
    (Get-ItemProperty -Path $entry -Name DisplayVersion).DisplayVersion
}

# Clicks the installer through with the defaults. Stops at a page named in
# $stopAt, and at a window of Cairn's uninstaller. Returns the pages seen and
# the exit code (null when stopped).
function Invoke-Interactive([string[]]$stopAt) {
    $p = Start-Process -FilePath $setup -PassThru
    $pages = [System.Collections.Generic.List[string]]::new()
    $clicked = @{}
    $deadline = (Get-Date).AddSeconds(120)
    try {
        while (-not $p.HasExited) {
            if ((Get-Date) -gt $deadline) { return @{ pages = $pages; stop = 'no end after 120 s' } }
            if ([Win]::Titles() | Where-Object { $_ -like 'Cairn Uninstall*' }) {
                return @{ pages = $pages; stop = 'a window of the uninstaller' }
            }
            $dialog = [Win]::DialogOf($p.Id)
            if ($dialog -eq [IntPtr]::Zero) { Start-Sleep -Milliseconds 200; continue }
            $texts = [Win]::Texts($dialog)
            $page = $titles | Where-Object { $texts -contains $_ } | Select-Object -First 1
            if ($page -and ($pages.Count -eq 0 -or $pages[-1] -ne $page)) { $pages.Add($page) }
            if ($stopAt -contains $page) { return @{ pages = $pages; stop = $page } }
            # Next once per page, once it has been enabled for a moment.
            $key = "$([Win]::Page($dialog))/$page"
            if ($page -and -not $clicked[$key] -and [Win]::NextEnabled($dialog)) {
                Start-Sleep -Milliseconds 300
                if ([Win]::NextEnabled($dialog)) {
                    if ($page -eq 'Completing Cairn Setup') {
                        Write-Host "  unticked: $([Win]::Untick([Win]::Page($dialog)) -join ', ')"
                    }
                    $clicked[$key] = $true
                    [Win]::Next($dialog)
                }
            }
            Start-Sleep -Milliseconds 200
        }
        return @{ pages = $pages; exit = $p.ExitCode }
    } finally {
        if (-not $p.HasExited) { $p.Kill() }
        # The uninstaller runs in place (_?=) or from a copy named Un.exe.
        Get-Process -Name 'uninstall', 'Un', 'Un_A' -ErrorAction SilentlyContinue | Stop-Process -Force
    }
}

# Runs the installer with `$flag`, which shows no page that waits; fails after 120 s.
function Invoke-Unattended([string]$flag) {
    $p = Start-Process -FilePath $setup -ArgumentList $flag -PassThru
    if (-not $p.WaitForExit(120000)) {
        $p.Kill()
        return 'no end after 120 s'
    }
    return $p.ExitCode
}

function Test-Data([string]$run) {
    foreach ($d in $data) {
        if (-not (Test-Path "$d\marker.txt")) { Fail "$run`: $d\marker.txt is gone" }
    }
}

Write-Host "Installer: $setup ($version)"
$code = Invoke-Unattended '/S'
if ($code -ne 0) { throw "the silent install ended with $code" }
foreach ($d in $data) {
    New-Item -ItemType Directory -Force -Path $d | Out-Null
    Set-Content -Path "$d\marker.txt" -Value 'kept'
}
$shell = New-Object -ComObject WScript.Shell
if (-not (Test-Path $startMenu)) { throw "no Start menu shortcut at $startMenu" }
$link = $shell.CreateShortcut($startMenu)
$link.Description = 'Changed by the update test'
$link.Save()

Write-Host '1. Interactively over an older version'
Set-Version '1.0.0'
$r = Invoke-Interactive @('Already Installed', 'Choose Install Location')
Write-Host "  pages: $($r.pages -join ' > ')"
if ($r.stop) {
    Fail "1: stopped at $($r.stop)"
    Set-Version $version
} else {
    if ($r.exit -ne 0) { Fail "1: exit code $($r.exit)" }
    if ((Get-Version) -ne $version) { Fail "1: the entry says $(Get-Version)" }
    if ($shell.CreateShortcut($startMenu).Description -ne 'Changed by the update test') {
        Fail '1: the Start menu shortcut was made again'
    }
}
Test-Data '1'

Write-Host '2. Interactively over the same version'
$r = Invoke-Interactive @()
Write-Host "  pages: $($r.pages -join ' > ')"
if ($r.stop) { Fail "2: stopped at $($r.stop)" }
elseif ($r.exit -ne 0) { Fail "2: exit code $($r.exit)" }
if ($r.pages -notcontains 'Already Installed') { Fail '2: no "Already Installed" page' }
Test-Data '2'

Write-Host '3. /P over an older version'
Set-Version '1.0.0'
$code = Invoke-Unattended '/P'
if ($code -ne 0) { Fail "3: $code" }
if ((Get-Version) -ne $version) { Fail "3: the entry says $(Get-Version)" }
Test-Data '3'

if ($failed.Count) { throw "$($failed.Count) checks failed: $($failed -join '; ')" }
Write-Host 'All updates kept the app data.'
