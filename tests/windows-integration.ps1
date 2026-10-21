param(
    [string]$Only = ""
)
# The Windows integration tests for the file path capture.
#
# Every case drives a real Explorer window with a real mouse gesture and then
# reads what the application logged. Nothing is mocked: the files come from a
# real folder on disk, the drag comes from real mouse events, and the answer
# comes from the real shell.
#
# The tests need a desktop to work on, and they move the mouse and press keys,
# so they must not be run while somebody else is using the machine.
#
#   powershell -ExecutionPolicy Bypass -File tests\windows-integration.ps1
#
# The program has to be built first:
#
#   cargo build --release

Add-Type -AssemblyName System.Windows.Forms, System.Drawing, UIAutomationClient, UIAutomationTypes

Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class Native {
    [DllImport("user32.dll")] public static extern void mouse_event(uint dwFlags, uint dx, uint dy, uint dwData, UIntPtr dwExtraInfo);
    [DllImport("user32.dll")] public static extern void keybd_event(byte bVk, byte bScan, uint dwFlags, UIntPtr dwExtraInfo);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    public const uint LEFTDOWN = 0x02, LEFTUP = 0x04, KEYUP = 0x02, MOVE = 0x01, ABSOLUTE = 0x8000;
}
"@

# The scratch folder the test files are put in, and the program under test.
$root = Join-Path $env:TEMP "open-tangerine-integration"
$exe = Join-Path $PSScriptRoot "..\target\release\open-tangerine.exe"
$exe = (Resolve-Path $exe -ErrorAction SilentlyContinue)
if (-not $exe) {
    Write-Output "the program has not been built yet: cargo build --release"
    exit 1
}
$exe = $exe.Path
New-Item -ItemType Directory -Force -Path $root | Out-Null
$report = Join-Path $root "integration-report.txt"
if (Test-Path $report) { Remove-Item -Force $report }

function Say($text) {
    $line = $text
    Add-Content -Path $report -Value $line
    Write-Output $line
}

function Get-ExplorerWindow($titleLike) {
    $root = [System.Windows.Automation.AutomationElement]::RootElement
    $condition = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ClassNameProperty, "CabinetWClass")
    $windows = $root.FindAll([System.Windows.Automation.TreeScope]::Children, $condition)
    foreach ($w in $windows) {
        if ($w.Current.Name -like $titleLike) { return $w }
    }
    return $null
}

function Get-ListItem($window, $name) {
    if (-not $window) { return $null }
    $items = $window.FindAll([System.Windows.Automation.TreeScope]::Descendants,
        (New-Object System.Windows.Automation.PropertyCondition(
            [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
            [System.Windows.Automation.ControlType]::ListItem)))
    # Explorer often shows the name without its extension.
    $short = [System.IO.Path]::GetFileNameWithoutExtension($name)
    foreach ($item in $items) {
        if ($item.Current.Name -eq $name -or $item.Current.Name -eq $short) { return $item }
    }
    return $null
}

# Finds an item and waits for it to turn up. A folder that is still being
# filled in has no items for a moment, and a test that clicks at (0, 0) would
# drag the background of the window instead of the file.
function Wait-ListItem($titlePattern, $name, [int]$Seconds = 10) {
    $deadline = (Get-Date).AddSeconds($Seconds)
    while ((Get-Date) -lt $deadline) {
        $window = Get-ExplorerWindow $titlePattern
        $item = Get-ListItem $window $name
        if ($item) { return $item }
        Start-Sleep -Milliseconds 250
    }
    return $null
}

function Minimize-Terminal {
    $terminal = Get-Process | Where-Object {
        $_.MainWindowTitle -ne "" -and $_.MainWindowHandle -ne 0 -and
        $_.ProcessName -match "WindowsTerminal|powershell|pwsh|cmd"
    } | Select-Object -First 1
    if ($terminal) {
        [void][Native]::ShowWindow($terminal.MainWindowHandle, 6)
        Start-Sleep -Milliseconds 300
    }
}

function Start-App($seconds) {
    $out = Join-Path $root "ot-stdout.txt"
    $err = Join-Path $root "ot-stderr.txt"
    Remove-Item -Force $out, $err -ErrorAction SilentlyContinue
    $env:OPEN_TANGERINE_LOG = "debug"
    $process = Start-Process -FilePath $exe -ArgumentList "--exit-after", "$seconds" -PassThru `
        -RedirectStandardError $err -RedirectStandardOutput $out
    Start-Sleep -Milliseconds 1200
    return $process
}

function Stop-App($process) {
    if ($process -and -not $process.HasExited) {
        $process.WaitForExit(8000) | Out-Null
        if (-not $process.HasExited) { $process.Kill() }
    }
    Start-Sleep -Milliseconds 400
    # Every case keeps its own log, so that one case cannot hide what
    # another one did.
    $err = Join-Path $root "ot-stderr.txt"
    if (Test-Path $err) {
        Copy-Item $err (Join-Path $root "ot-stderr-$script:CaseName.txt") -Force
    }
}

function Get-AppLog {
    $err = Join-Path $root "ot-stderr.txt"
    if (Test-Path $err) { return Get-Content $err }
    return @()
}

# Presses on a file, drags a little, optionally holds Shift (which opens the
# wheel), optionally chooses a segment, and lets go.
function Invoke-Gesture($x, $y, [bool]$holdShift, [bool]$choose, [int]$dx = 120, [int]$dy = 60) {
    [void][Native]::SetCursorPos($x, $y)
    Start-Sleep -Milliseconds 400
    [Native]::mouse_event([Native]::LEFTDOWN, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 350
    for ($i = 1; $i -le 10; $i++) {
        [void][Native]::SetCursorPos(($x + [int]($dx * $i / 10)), ($y + [int]($dy * $i / 10)))
        Start-Sleep -Milliseconds 45
    }
    if ($holdShift) {
        [Native]::keybd_event(0x10, 0, 0, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 700
        for ($i = 11; $i -le 20; $i++) {
            [void][Native]::SetCursorPos(($x + [int]($dx * $i / 10)), ($y + [int]($dy * $i / 10)))
            Start-Sleep -Milliseconds 55
        }
        Start-Sleep -Milliseconds 500
    }
    if ($choose) {
        # Move onto a segment of the wheel, which lies around the pointer.
        # The directions are tried one after the other until the log says
        # that a segment is under the pointer.
        $centreX = $x + [int]($dx)
        $centreY = $y + [int]($dy)
        $found = $false
        foreach ($angle in @(90, 135, 45, 0, 180, 270, 315, 225)) {
            $radius = 110
            $rad = $angle * [Math]::PI / 180
            $tx = [int]($centreX + $radius * [Math]::Cos($rad))
            $ty = [int]($centreY - $radius * [Math]::Sin($rad))
            [void][Native]::SetCursorPos($tx, $ty)
            Start-Sleep -Milliseconds 350
            $tail = Get-AppLog | Where-Object { $_ -match "the pointer is over" } | Select-Object -Last 1
            if ($tail -match "segment Some\((\d+)\)") { $found = $true; break }
        }
        if (-not $found) { Write-Output "  (no segment was found under the pointer)" }
        Start-Sleep -Milliseconds 400
    }
    if ($holdShift) {
        [Native]::keybd_event(0x10, 0, [Native]::KEYUP, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 300
    }
    [Native]::mouse_event([Native]::LEFTUP, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 900
}

function Select-Item($titlePattern, $name, [bool]$control) {
    $item = Wait-ListItem $titlePattern $name
    if ($null -eq $item) { throw "the item '$name' is not in the window" }
    $rect = $item.Current.BoundingRectangle
    if ($rect.Width -le 0 -or $rect.Height -le 0) { throw "the item '$name' has no size" }
    $x = [int]($rect.X + 25)
    $y = [int]($rect.Y + $rect.Height / 2)
    if ($x -le 0 -or $y -le 0) { throw "the item '$name' is not where it should be" }
    [void][Native]::SetCursorPos($x, $y)
    Start-Sleep -Milliseconds 250
    if ($control) {
        [Native]::keybd_event(0x11, 0, 0, [UIntPtr]::Zero)   # Control down
        Start-Sleep -Milliseconds 150
    }
    [Native]::mouse_event([Native]::LEFTDOWN, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 120
    [Native]::mouse_event([Native]::LEFTUP, 0, 0, 0, [UIntPtr]::Zero)
    if ($control) {
        [Native]::keybd_event(0x11, 0, [Native]::KEYUP, [UIntPtr]::Zero)
    }
    Start-Sleep -Milliseconds 400
    return @($x, $y)
}

# ------------------------------------------------------------------ the cases

# The name of the case that is running, so that every case keeps its own log.
$script:CaseName = "case"

function Case-OneFile {
    $script:CaseName = "one"
    Say ""
    Say "=== case 1: one file ==="
    $folder = Join-Path $root "itest-one"
    if (Test-Path $folder) { Remove-Item -Recurse -Force $folder }
    New-Item -ItemType Directory -Force -Path $folder | Out-Null
    Set-Content (Join-Path $folder "only.txt") "hello"

    Get-Process explorer -ErrorAction SilentlyContinue | ForEach-Object { $_.CloseMainWindow() | Out-Null }
    Start-Sleep -Seconds 2
    Minimize-Terminal
    Start-Process explorer.exe -ArgumentList "/e,`"$folder`""
    Start-Sleep -Seconds 4

    $window = Get-ExplorerWindow "itest-one*"
    if ($null -eq $window) { Say "FAIL: the Explorer window did not open"; return }
    $hwnd = [IntPtr]$window.Current.NativeWindowHandle
    [void][Native]::ShowWindow($hwnd, 3)
    [void][Native]::SetForegroundWindow($hwnd)
    Start-Sleep -Seconds 2
    Minimize-Terminal

    $window = Get-ExplorerWindow "itest-one*"
    $at = Select-Item "itest-one*" "only.txt" $false
    Minimize-Terminal
    [void][Native]::SetForegroundWindow($hwnd)
    Start-Sleep -Milliseconds 500

    $process = Start-App 22
    Minimize-Terminal
    [void][Native]::SetForegroundWindow($hwnd)
    Start-Sleep -Milliseconds 500
    Invoke-Gesture $at[0] $at[1] $true $true
    Start-Sleep -Seconds 2
    Stop-App $process

    $log = Get-AppLog
    $start = $log | Select-String -Pattern "drag \d+ started in the Explorer window" | Select-Object -First 1
    $action = $log | Select-String -Pattern "placeholder:" | Select-Object -First 1
    $agrees = $log | Select-String -Pattern "the drop target agrees" | Select-Object -First 1
    $mismatch = $log | Select-String -Pattern "the drop target reports" | Select-Object -First 1
    if ($start) { Say ("the log says: " + $start.Line) } else { Say "FAIL: the log does not say that a drag started" }
    if ($action) { Say ("the log says: " + $action.Line) } else { Say "FAIL: no action was logged" }
    if ($agrees) { Say ("the log says: " + $agrees.Line) }
    if ($mismatch) { Say ("FAIL: the shell and the drop target disagree: " + $mismatch.Line) }

    # The shell is asked directly what it says, so that the answer the
    # application used can be compared with the truth.
    $shell = New-Object -ComObject Shell.Application
    $truth = @()
    foreach ($w in $shell.Windows()) {
        if ([int64]$w.HWND -ne [int64]$hwnd) { continue }
        foreach ($it in $w.Document.SelectedItems()) { $truth += $it.Path }
    }
    Say ("the shell itself reports: " + ($truth -join ", "))
    $expected = Join-Path $folder "only.txt"
    if ($truth.Count -eq 1 -and $truth[0] -eq $expected) {
        Say "PASS: the file that is being dragged is the file that was dragged"
    } else {
        Say "FAIL: the shell reports something else"
    }

    if ($start -and $start.Line -match "1 file\(s\)" -and $action -and $action.Line -match "1 file\(s\)" -and -not $mismatch) {
        Say "PASS: one file was captured"
    } else {
        Say "FAIL: the file count was not one"
    }
}

function Case-MultipleFiles {
    $script:CaseName = "many"
    Say ""
    Say "=== case 2: several files ==="
    $folder = Join-Path $root "itest-many"
    if (Test-Path $folder) { Remove-Item -Recurse -Force $folder }
    New-Item -ItemType Directory -Force -Path $folder | Out-Null
    1..3 | ForEach-Object { Set-Content (Join-Path $folder "file$_.txt") "hello" }

    Get-Process explorer -ErrorAction SilentlyContinue | ForEach-Object { $_.CloseMainWindow() | Out-Null }
    Start-Sleep -Seconds 2
    Minimize-Terminal
    Start-Process explorer.exe -ArgumentList "/e,`"$folder`""
    Start-Sleep -Seconds 4

    $window = Get-ExplorerWindow "itest-many*"
    if ($null -eq $window) { Say "FAIL: the Explorer window did not open"; return }
    $hwnd = [IntPtr]$window.Current.NativeWindowHandle
    [void][Native]::ShowWindow($hwnd, 3)
    [void][Native]::SetForegroundWindow($hwnd)
    Start-Sleep -Seconds 2
    Minimize-Terminal

    $window = Get-ExplorerWindow "itest-many*"
    $at = Select-Item "itest-many*" "file1.txt" $false
    foreach ($name in @("file2.txt", "file3.txt")) {
        $at = Select-Item "itest-many*" $name $true
    }
    Minimize-Terminal
    [void][Native]::SetForegroundWindow($hwnd)
    Start-Sleep -Milliseconds 500

    $process = Start-App 22
    Minimize-Terminal
    [void][Native]::SetForegroundWindow($hwnd)
    Start-Sleep -Milliseconds 500
    Invoke-Gesture $at[0] $at[1] $true $true
    Start-Sleep -Seconds 2
    Stop-App $process

    $log = Get-AppLog
    $start = $log | Select-String -Pattern "drag \d+ started in the Explorer window" | Select-Object -First 1
    $action = $log | Select-String -Pattern "placeholder:" | Select-Object -First 1
    if ($start) { Say ("the log says: " + $start.Line) } else { Say "FAIL: the log does not say that a drag started" }
    if ($action) { Say ("the log says: " + $action.Line) } else { Say "FAIL: no action was logged" }
    if ($start -and $start.Line -match "3 file\(s\)" -and $action -and $action.Line -match "3 file\(s\)") {
        Say "PASS: three files were captured"
    } else {
        Say "FAIL: the file count was not three"
    }
}

function Case-Consecutive {
    $script:CaseName = "consecutive"
    Say ""
    Say "=== case 3: one drag after another, with different files ==="
    $folder = Join-Path $root "itest-consecutive"
    if (Test-Path $folder) { Remove-Item -Recurse -Force $folder }
    New-Item -ItemType Directory -Force -Path $folder | Out-Null
    Set-Content (Join-Path $folder "first.txt") "hello"
    Set-Content (Join-Path $folder "second.txt") "hello"
    Set-Content (Join-Path $folder "third.txt") "hello"

    Get-Process explorer -ErrorAction SilentlyContinue | ForEach-Object { $_.CloseMainWindow() | Out-Null }
    Start-Sleep -Seconds 2
    Minimize-Terminal
    Start-Process explorer.exe -ArgumentList "/e,`"$folder`""
    Start-Sleep -Seconds 4

    $window = Get-ExplorerWindow "itest-consecutive*"
    if ($null -eq $window) { Say "FAIL: the Explorer window did not open"; return }
    $hwnd = [IntPtr]$window.Current.NativeWindowHandle
    [void][Native]::ShowWindow($hwnd, 3)
    [void][Native]::SetForegroundWindow($hwnd)
    Start-Sleep -Seconds 2
    Minimize-Terminal

    $process = Start-App 45
    Minimize-Terminal
    [void][Native]::SetForegroundWindow($hwnd)
    Start-Sleep -Milliseconds 500

    # The first drag: first.txt and second.txt, and the drag is cancelled by
    # letting go of the mouse over the hub of the wheel.
    $window = Get-ExplorerWindow "itest-consecutive*"
    $first = Select-Item "itest-consecutive*" "first.txt" $false
    $second = Select-Item "itest-consecutive*" "second.txt" $true
    Minimize-Terminal
    [void][Native]::SetForegroundWindow($hwnd)
    Start-Sleep -Milliseconds 400
    Invoke-Gesture $first[0] $first[1] $true $false 40 20
    Start-Sleep -Milliseconds 800

    # The wheel is still up after a drag that missed, and it takes the clicks
    # that land on it, so it is closed with Escape before the next file is
    # picked.
    [Native]::keybd_event(0x1B, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 200
    [Native]::keybd_event(0x1B, 0, [Native]::KEYUP, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 800

    # The second drag: third.txt alone. The count must be one, and not the
    # two files of the drag before it.
    $window = Get-ExplorerWindow "itest-consecutive*"
    $third = Select-Item "itest-consecutive*" "third.txt" $false
    Minimize-Terminal
    [void][Native]::SetForegroundWindow($hwnd)
    Start-Sleep -Milliseconds 400
    Invoke-Gesture $third[0] $third[1] $true $true
    Start-Sleep -Seconds 2
    Stop-App $process

    $log = Get-AppLog
    $starts = @($log | Select-String -Pattern "drag \d+ started in the Explorer window")
    $action = $log | Select-String -Pattern "placeholder:" | Select-Object -First 1
    foreach ($line in $starts) { Say ("the log says: " + $line.Line) }
    if ($action) { Say ("the log says: " + $action.Line) } else { Say "FAIL: no action was logged" }
    if ($starts.Count -ge 2 -and $starts[0].Line -match "2 file\(s\)" -and $starts[1].Line -match "1 file\(s\)" -and $action -and $action.Line -match "1 file\(s\)") {
        Say "PASS: the second drag did not keep the files of the first"
    } else {
        Say "FAIL: the drags did not have their own files"
    }
}

function Case-Cancelled {
    $script:CaseName = "cancel"
    Say ""
    Say "=== case 4: a drag that is cancelled ==="
    $folder = Join-Path $root "itest-cancel"
    if (Test-Path $folder) { Remove-Item -Recurse -Force $folder }
    New-Item -ItemType Directory -Force -Path $folder | Out-Null
    Set-Content (Join-Path $folder "cancelled.txt") "hello"

    Get-Process explorer -ErrorAction SilentlyContinue | ForEach-Object { $_.CloseMainWindow() | Out-Null }
    Start-Sleep -Seconds 2
    Minimize-Terminal
    Start-Process explorer.exe -ArgumentList "/e,`"$folder`""
    Start-Sleep -Seconds 4

    $window = Get-ExplorerWindow "itest-cancel*"
    if ($null -eq $window) { Say "FAIL: the Explorer window did not open"; return }
    $hwnd = [IntPtr]$window.Current.NativeWindowHandle
    [void][Native]::ShowWindow($hwnd, 3)
    [void][Native]::SetForegroundWindow($hwnd)
    Start-Sleep -Seconds 2
    Minimize-Terminal

    $window = Get-ExplorerWindow "itest-cancel*"
    $at = Select-Item "itest-cancel*" "cancelled.txt" $false
    Minimize-Terminal
    [void][Native]::SetForegroundWindow($hwnd)
    Start-Sleep -Milliseconds 500

    $process = Start-App 25
    Minimize-Terminal
    [void][Native]::SetForegroundWindow($hwnd)
    Start-Sleep -Milliseconds 500

    # The drag starts, the wheel opens, and then Escape is pressed: the
    # interaction is cancelled and nothing is run.
    [void][Native]::SetCursorPos($at[0], $at[1])
    Start-Sleep -Milliseconds 400
    [Native]::mouse_event([Native]::LEFTDOWN, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 350
    for ($i = 1; $i -le 10; $i++) {
        [void][Native]::SetCursorPos(($at[0] + $i * 4), ($at[1] + $i * 3))
        Start-Sleep -Milliseconds 45
    }
    [Native]::keybd_event(0x10, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 900
    [Native]::keybd_event(0x1B, 0, 0, [UIntPtr]::Zero)      # Escape
    Start-Sleep -Milliseconds 200
    [Native]::keybd_event(0x1B, 0, [Native]::KEYUP, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 400
    [Native]::keybd_event(0x10, 0, [Native]::KEYUP, [UIntPtr]::Zero)
    [Native]::mouse_event([Native]::LEFTUP, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Seconds 2
    Stop-App $process

    $log = Get-AppLog
    $start = $log | Select-String -Pattern "drag \d+ started in the Explorer window" | Select-Object -First 1
    $action = $log | Select-String -Pattern "placeholder:" | Select-Object -First 1
    if ($start) { Say ("the log says: " + $start.Line) } else { Say "FAIL: the log does not say that a drag started" }
    if ($action) { Say ("FAIL: something was run: " + $action.Line) } else { Say "PASS: nothing was run" }
}

function Case-Unselected {
    $script:CaseName = "unselected"
    Say ""
    Say "=== case 6: dragging a file that was not selected before ==="
    $folder = Join-Path $root "itest-unselected"
    if (Test-Path $folder) { Remove-Item -Recurse -Force $folder }
    New-Item -ItemType Directory -Force -Path $folder | Out-Null
    Set-Content (Join-Path $folder "one.txt") "hello"
    Set-Content (Join-Path $folder "two.txt") "hello"
    Set-Content (Join-Path $folder "three.txt") "hello"

    Get-Process explorer -ErrorAction SilentlyContinue | ForEach-Object { $_.CloseMainWindow() | Out-Null }
    Start-Sleep -Seconds 2
    Minimize-Terminal
    Start-Process explorer.exe -ArgumentList "/e,`"$folder`""
    Start-Sleep -Seconds 4

    $window = Get-ExplorerWindow "itest-unselected*"
    if ($null -eq $window) { Say "FAIL: the Explorer window did not open"; return }
    $hwnd = [IntPtr]$window.Current.NativeWindowHandle
    [void][Native]::ShowWindow($hwnd, 3)
    [void][Native]::SetForegroundWindow($hwnd)
    Start-Sleep -Seconds 2
    Minimize-Terminal

    # Two files are selected, and then a third one that was not selected
    # before is dragged. Only the third one is carried by the drag.
    $window = Get-ExplorerWindow "itest-unselected*"
    $one = Select-Item "itest-unselected*" "one.txt" $false
    $two = Select-Item "itest-unselected*" "two.txt" $true
    Minimize-Terminal
    [void][Native]::SetForegroundWindow($hwnd)
    Start-Sleep -Milliseconds 400
    $three = Select-Item "itest-unselected*" "three.txt" $false
    Minimize-Terminal
    [void][Native]::SetForegroundWindow($hwnd)
    Start-Sleep -Milliseconds 400

    $process = Start-App 22
    Minimize-Terminal
    [void][Native]::SetForegroundWindow($hwnd)
    Start-Sleep -Milliseconds 500
    Invoke-Gesture $three[0] $three[1] $true $true
    Start-Sleep -Seconds 2
    Stop-App $process

    $log = Get-AppLog
    $start = $log | Select-String -Pattern "drag \d+ started in the Explorer window" | Select-Object -First 1
    $action = $log | Select-String -Pattern "placeholder:" | Select-Object -First 1
    $mismatch = $log | Select-String -Pattern "the drop target reports" | Select-Object -First 1
    if ($start) { Say ("the log says: " + $start.Line) } else { Say "FAIL: the log does not say that a drag started" }
    if ($action) { Say ("the log says: " + $action.Line) } else { Say "FAIL: no action was logged" }
    if ($mismatch) { Say ("FAIL: the shell and the drop target disagree: " + $mismatch.Line) }
    if ($start -and $start.Line -match "1 file\(s\)" -and $action -and $action.Line -match "1 file\(s\)" -and -not $mismatch) {
        Say "PASS: only the file that was dragged was carried"
    } else {
        Say "FAIL: the wrong number of files was carried"
    }
}

function Case-WheelStays {
    $script:CaseName = "stays"
    Say ""
    Say "=== case 7: the wheel does not move when the drag is released ==="
    $folder = Join-Path $root "itest-stays"
    if (Test-Path $folder) { Remove-Item -Recurse -Force $folder }
    New-Item -ItemType Directory -Force -Path $folder | Out-Null
    Set-Content (Join-Path $folder "stays.txt") "hello"

    Get-Process explorer -ErrorAction SilentlyContinue | ForEach-Object { $_.CloseMainWindow() | Out-Null }
    Start-Sleep -Seconds 2
    Minimize-Terminal
    Start-Process explorer.exe -ArgumentList "/e,`"$folder`""
    Start-Sleep -Seconds 4

    $window = Get-ExplorerWindow "itest-stays*"
    if ($null -eq $window) { Say "FAIL: the Explorer window did not open"; return }
    $hwnd = [IntPtr]$window.Current.NativeWindowHandle
    [void][Native]::ShowWindow($hwnd, 3)
    [void][Native]::SetForegroundWindow($hwnd)
    Start-Sleep -Seconds 2
    Minimize-Terminal

    $at = Select-Item "itest-stays*" "stays.txt" $false
    Minimize-Terminal
    [void][Native]::SetForegroundWindow($hwnd)
    Start-Sleep -Milliseconds 500

    $process = Start-App 25
    Minimize-Terminal
    [void][Native]::SetForegroundWindow($hwnd)
    Start-Sleep -Milliseconds 500

    # The wheel opens, and the drag is released over a segment of it.
    [void][Native]::SetCursorPos($at[0], $at[1])
    Start-Sleep -Milliseconds 500
    [Native]::mouse_event([Native]::LEFTDOWN, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 350
    for ($i = 1; $i -le 10; $i++) {
        [void][Native]::SetCursorPos(($at[0] + $i * 4), ($at[1] + $i * 3))
        Start-Sleep -Milliseconds 45
    }
    [Native]::keybd_event(0x10, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 900
    for ($i = 11; $i -le 18; $i++) {
        [void][Native]::SetCursorPos(($at[0] + $i * 4), ($at[1] + $i * 3))
        Start-Sleep -Milliseconds 55
    }
    Start-Sleep -Milliseconds 700

    # Onto a segment of the wheel: up and to the right of the pointer.
    $centreX = $at[0] + 18 * 4
    $centreY = $at[1] + 18 * 3
    [void][Native]::SetCursorPos(($centreX + 30), ($centreY - 95))
    Start-Sleep -Milliseconds 800

    [Native]::keybd_event(0x10, 0, [Native]::KEYUP, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 200
    [Native]::mouse_event([Native]::LEFTUP, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Seconds 2
    Stop-App $process

    $log = Get-AppLog
    $moves = @($log | Select-String -Pattern "the window was moved to")
    $drop = $log | Select-String -Pattern "the drag was dropped on the wheel" | Select-Object -First 1
    $action = $log | Select-String -Pattern "placeholder:" | Select-Object -First 1
    foreach ($line in $moves) { Say ("the log says: " + $line.Line) }
    if ($drop) { Say ("the log says: " + $drop.Line) } else { Say "FAIL: the drop was not seen" }
    if ($action) { Say ("the log says: " + $action.Line) } else { Say "FAIL: no action was logged" }

    # The wheel is put in one place when it opens, and it must not be moved
    # again: not when the drag is released, and not while it fades away.
    if ($moves.Count -eq 1) {
        Say "PASS: the wheel was moved once, when it opened, and never again"
    } else {
        Say "FAIL: the wheel was moved $($moves.Count) times"
    }
    if ($drop -and $action) {
        Say "PASS: the drop chose something and the action ran on it"
    } else {
        Say "FAIL: the release did not choose anything"
    }
}

function Case-DropOnHub {
    $script:CaseName = "hub"
    Say ""
    Say "=== case 8: a drag that is released on the hub ==="
    $folder = Join-Path $root "itest-hub"
    if (Test-Path $folder) { Remove-Item -Recurse -Force $folder }
    New-Item -ItemType Directory -Force -Path $folder | Out-Null
    Set-Content (Join-Path $folder "hub.txt") "hello"

    Get-Process explorer -ErrorAction SilentlyContinue | ForEach-Object { $_.CloseMainWindow() | Out-Null }
    Start-Sleep -Seconds 2
    Minimize-Terminal
    Start-Process explorer.exe -ArgumentList "/e,`"$folder`""
    Start-Sleep -Seconds 4

    $window = Get-ExplorerWindow "itest-hub*"
    if ($null -eq $window) { Say "FAIL: the Explorer window did not open"; return }
    $hwnd = [IntPtr]$window.Current.NativeWindowHandle
    [void][Native]::ShowWindow($hwnd, 3)
    [void][Native]::SetForegroundWindow($hwnd)
    Start-Sleep -Seconds 2
    Minimize-Terminal

    $at = Select-Item "itest-hub*" "hub.txt" $false
    Minimize-Terminal
    [void][Native]::SetForegroundWindow($hwnd)
    Start-Sleep -Milliseconds 500

    $process = Start-App 25
    Minimize-Terminal
    [void][Native]::SetForegroundWindow($hwnd)
    Start-Sleep -Milliseconds 500

    # The wheel opens around the pointer, and the drag is released on the hub
    # in the middle of it: there is no choice there.
    [void][Native]::SetCursorPos($at[0], $at[1])
    Start-Sleep -Milliseconds 500
    [Native]::mouse_event([Native]::LEFTDOWN, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 350
    for ($i = 1; $i -le 10; $i++) {
        [void][Native]::SetCursorPos(($at[0] + $i * 4), ($at[1] + $i * 3))
        Start-Sleep -Milliseconds 45
    }
    [Native]::keybd_event(0x10, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 900
    for ($i = 11; $i -le 18; $i++) {
        [void][Native]::SetCursorPos(($at[0] + $i * 4), ($at[1] + $i * 3))
        Start-Sleep -Milliseconds 55
    }
    Start-Sleep -Milliseconds 700

    # Back to the middle of the wheel: the hub.
    [void][Native]::SetCursorPos(($at[0] + 18 * 4), ($at[1] + 18 * 3))
    Start-Sleep -Milliseconds 800

    [Native]::keybd_event(0x10, 0, [Native]::KEYUP, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 200
    [Native]::mouse_event([Native]::LEFTUP, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Seconds 2
    Stop-App $process

    $log = Get-AppLog
    $drop = $log | Select-String -Pattern "the drop landed at" | Select-Object -First 1
    $action = $log | Select-String -Pattern "placeholder:" | Select-Object -First 1
    if ($drop) { Say ("the log says: " + $drop.Line) } else { Say "FAIL: the drop was not seen" }
    if ($action) { Say ("FAIL: something was run: " + $action.Line) } else { Say "PASS: nothing was run" }
}

function Case-Unsupported {
    $script:CaseName = "unsupported"
    Say ""
    Say "=== case 5: a drag that does not come from Explorer ==="
    # Every Explorer window is closed, so that the point the drag starts at
    # really is on the desktop and not on a file list.
    Get-Process explorer -ErrorAction SilentlyContinue | ForEach-Object { $_.CloseMainWindow() | Out-Null }
    Start-Sleep -Seconds 3

    $process = Start-App 18
    Minimize-Terminal

    # The desktop is a shell window, but not a file list, so a press on it
    # must never open the wheel.
    [void][Native]::SetCursorPos(40, 40)
    Start-Sleep -Milliseconds 500
    [Native]::mouse_event([Native]::LEFTDOWN, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 300
    for ($i = 1; $i -le 12; $i++) {
        [void][Native]::SetCursorPos((40 + $i * 10), (40 + $i * 8))
        Start-Sleep -Milliseconds 45
    }
    [Native]::keybd_event(0x10, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 700
    [Native]::keybd_event(0x10, 0, [Native]::KEYUP, [UIntPtr]::Zero)
    [Native]::mouse_event([Native]::LEFTUP, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Seconds 2
    Stop-App $process

    $log = Get-AppLog
    $rejected = $log | Select-String -Pattern "was not a file drag" | Select-Object -First 1
    $started = $log | Select-String -Pattern "drag \d+ started in the Explorer window" | Select-Object -First 1
    if ($rejected) { Say ("the log says: " + $rejected.Line) } else { Say "FAIL: the press was not reported as rejected" }
    if ($started) { Say ("FAIL: a drag was started anyway: " + $started.Line) } else { Say "PASS: no drag was started" }
}

Say "the Windows integration tests for the file path capture"
Say ("the application is at " + $exe)

# Only one wheel at a time: a copy of the program that is still running would
# keep the wheel to itself, and the tests would see nothing.
Get-Process open-tangerine -ErrorAction SilentlyContinue | ForEach-Object {
    $_.Kill()
    $_.WaitForExit(5000) | Out-Null
}
Start-Sleep -Milliseconds 500

if ($Only -eq "" -or $Only -eq "one") { Case-OneFile }
if ($Only -eq "" -or $Only -eq "many") { Case-MultipleFiles }
if ($Only -eq "" -or $Only -eq "consecutive") { Case-Consecutive }
if ($Only -eq "" -or $Only -eq "cancel") { Case-Cancelled }
if ($Only -eq "" -or $Only -eq "unselected") { Case-Unselected }
if ($Only -eq "" -or $Only -eq "stays") { Case-WheelStays }
if ($Only -eq "" -or $Only -eq "hub") { Case-DropOnHub }
if ($Only -eq "" -or $Only -eq "unsupported") { Case-Unsupported }

Say ""
Say "the report is at $report"
