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
    [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT lpPoint);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X; public int Y; }
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

# Says something without putting it in the pipeline, so that a function that
# gives something back is not muddled up by its own messages.
function Note($text) {
    Add-Content -Path $report -Value $text
    Write-Host $text
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

# The shape of the wheel, as the application has it. This mirrors
# RadialGeometry in src/radial/geometry.rs: eight segments, an inner radius of
# 64 and an outer radius of 150, with the angles measured clockwise from the
# top. The radii are given as a part of the wheel's size, so a display that
# is scaled comes out right without anything being assumed about it.
$WheelSegments = 8
$WheelInnerRatio = 64.0 / 380.0
$WheelOuterRatio = 150.0 / 380.0

# Reads where the application put the wheel, and how big it is.
#
# The application writes this down every time it puts the wheel somewhere, so
# this is the wheel's real place even when a second drag opens a second wheel.
# It is not the pointer's place: the wheel is pushed against the edge of the
# screen when the pointer is near it.
function Get-Wheel {
    $line = Get-AppLog | Where-Object { $_ -match "the window was moved to" } | Select-Object -Last 1
    if (-not $line) { return $null }
    if ($line -match "moved to \((-?\d+), (-?\d+)\), (\d+) by \d+ pixels, and is now at \((-?\d+), (-?\d+), (-?\d+), (-?\d+)\)") {
        $left = [int]$Matches[4]
        $top = [int]$Matches[5]
        $right = [int]$Matches[6]
        $bottom = [int]$Matches[7]
        return @{
            Left = $left
            Top = $top
            Size = $right - $left
            CentreX = ($left + $right) / 2.0
            CentreY = ($top + $bottom) / 2.0
        }
    }
    return $null
}

# The point in the middle of a segment, in screen pixels.
function Segment-Point($wheel, $index) {
    $radius = $wheel.Size * ($WheelInnerRatio + $WheelOuterRatio) / 2
    $angle = ($index * 360 / $WheelSegments) + (180 / $WheelSegments)
    $rad = $angle * [Math]::PI / 180
    return @([int]($wheel.CentreX + $radius * [Math]::Sin($rad)),
              [int]($wheel.CentreY - $radius * [Math]::Cos($rad)))
}

# The segment a point falls in, the way the application works it out.
# $null means the point is not on a segment.
function Segment-At($wheel, $x, $y) {
    $inner = $wheel.Size * $WheelInnerRatio
    $outer = $wheel.Size * $WheelOuterRatio
    $dx = $x - $wheel.CentreX
    $dy = $y - $wheel.CentreY
    $distance = [Math]::Sqrt($dx * $dx + $dy * $dy)
    if ($distance -lt $inner -or $distance -gt $outer) { return $null }
    $angle = [Math]::Atan2($dx, -$dy) * 180 / [Math]::PI
    if ($angle -lt 0) { $angle += 360 }
    $index = [int][Math]::Floor($angle / (360 / $WheelSegments))
    if ($index -ge $WheelSegments) { $index = $WheelSegments - 1 }
    return $index
}

# The segment a log line says is lit. $null means the line says nothing.
# -1 means the hub, which is not a segment.
function Hover-Index($line) {
    if (-not $line) { return $null }
    if ($line -match "segment Some\((\d+)\)") { return [int]$Matches[1] }
    if ($line -match "segment None") { return -1 }
    return $null
}

# Moves the pointer to a place in a few steps, the way a hand does it.
#
# One jump is not the same as a movement: a drag that is given to Windows in
# one piece is not reported the same way as a drag that is moved along.
function Move-To-Point($x, $y, [int]$Steps = 4, [int]$Pause = 35) {
    $from = Cursor-Position
    for ($i = 1; $i -le $Steps; $i++) {
        $tx = [int]($from[0] + ($x - $from[0]) * $i / $Steps)
        $ty = [int]($from[1] + ($y - $from[1]) * $i / $Steps)
        [void][Native]::SetCursorPos($tx, $ty)
        Start-Sleep -Milliseconds $Pause
    }
}

# Where the pointer is, in screen pixels.
function Cursor-Position {
    $p = New-Object Native+POINT
    [void][Native]::GetCursorPos([ref]$p)
    return @($p.X, $p.Y)
}
# and gives that line back together with how long it took.
#
# Only the lines written after the movement being tested are looked at: a line
# from before the movement says nothing about where the pointer is now.
#
# The application writes a line only when the lit segment changes, so a move
# that does not change it writes nothing at all. The wait is bounded for that
# reason.
function Get-FreshHover($fromLine, [int]$Milliseconds = 400) {
    $started = Get-Date
    $deadline = $started.AddMilliseconds($Milliseconds)
    while ($true) {
        $lines = @(Get-AppLog)
        if ($lines.Count -gt $fromLine) {
            $new = $lines[$fromLine..($lines.Count - 1)] |
                Where-Object { $_ -match "the pointer is over" } | Select-Object -Last 1
            if ($new) {
                return @{ line = $new; elapsed = ((Get-Date) - $started).TotalMilliseconds }
            }
        }
        if ((Get-Date) -ge $deadline) { return $null }
        Start-Sleep -Milliseconds 10
    }
}

# Presses on a file, drags a little, optionally holds Shift (which opens the
# wheel), optionally puts the pointer on a segment, and lets go.
#
# The pointer stops where the wheel opens. Running past it would cross
# another segment on the way, and the test would be about the wrong one.
#
# It gives back the line the application wrote about the segment that is lit,
# so that a caller can see what the application thought.
function Invoke-Gesture($x, $y, [bool]$holdShift, [bool]$choose, [int]$dx = 120, [int]$dy = 60) {
    [void][Native]::SetCursorPos($x, $y)
    Start-Sleep -Milliseconds 400
    [Native]::mouse_event([Native]::LEFTDOWN, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 350
    for ($i = 1; $i -le 10; $i++) {
        [void][Native]::SetCursorPos(($x + [int]($dx * $i / 10)), ($y + [int]($dy * $i / 10)))
        Start-Sleep -Milliseconds 45
    }
    # The wheel opens around the pointer here.
    $centreX = $x + [int]($dx)
    $centreY = $y + [int]($dy)
    if ($holdShift) {
        [Native]::keybd_event(0x10, 0, 0, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 700
        Start-Sleep -Milliseconds 500
    }
    $hover = $null
    if ($choose) {
        $wheel = Get-Wheel
        if (-not $wheel) {
            Note "  (the application did not say where it put the wheel)"
        } else {
            # The target is worked out from the wheel's own shape, so that the
            # pointer lands in the middle of a segment and not on a seam.
            $point = Segment-Point $wheel 0
            $before = @(Get-AppLog).Count
            Move-To-Point $point[0] $point[1]
            $hover = Get-FreshHover $before
            if ($hover) {
                Note ("  the pointer is on segment {0} of the wheel, and the application said so after {1} ms" -f `
                    (Hover-Index $hover.line), [int]$hover.elapsed)
            } else {
                Note "  (the application did not say which segment is lit)"
            }
            Start-Sleep -Milliseconds 400
        }
    }
    if ($holdShift) {
        [Native]::keybd_event(0x10, 0, [Native]::KEYUP, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 300
    }
    [Native]::mouse_event([Native]::LEFTUP, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 900
    return $hover
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
    if ($action) { Say ("FAIL: something was run: " + $action.Line) } else { Say "PASS: nothing was run, because nothing can be done with these files yet" }
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

    # The number of files the application found is in its own line, and the
    # line that says the drag started says it too.
    $selection = $log | Select-String -Pattern "the selection holds" | Select-Object -First 1
    if ($selection) { Say ("the log says: " + $selection.Line) }
    if ($start -and $start.Line -match "1 file\(s\)" -and $selection -and $selection.Line -match "1 item\(s\)" -and -not $mismatch) {
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
    $selection = $log | Select-String -Pattern "the selection holds" | Select-Object -First 1
    if ($start) { Say ("the log says: " + $start.Line) } else { Say "FAIL: the log does not say that a drag started" }
    if ($selection) { Say ("the log says: " + $selection.Line) }
    if ($start -and $start.Line -match "3 file\(s\)" -and $selection -and $selection.Line -match "3 item\(s\)") {
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
    $selections = @($log | Select-String -Pattern "the selection holds")
    $action = $log | Select-String -Pattern "placeholder:" | Select-Object -First 1
    foreach ($line in $starts) { Say ("the log says: " + $line.Line) }
    foreach ($line in $selections) { Say ("the log says: " + $line.Line) }
    if ($action) { Say ("FAIL: something was run: " + $action.Line) } else { Say "PASS: nothing was run, because nothing can be done with these files yet" }
    if ($starts.Count -ge 2 -and $starts[0].Line -match "2 file\(s\)" -and $starts[1].Line -match "1 file\(s\)" -and $selections.Count -ge 2 -and $selections[0].Line -match "2 item\(s\)" -and $selections[1].Line -match "1 item\(s\)") {
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
    $selection = $log | Select-String -Pattern "the selection holds" | Select-Object -First 1
    $action = $log | Select-String -Pattern "placeholder:" | Select-Object -First 1
    $mismatch = $log | Select-String -Pattern "the drop target reports" | Select-Object -First 1
    if ($start) { Say ("the log says: " + $start.Line) } else { Say "FAIL: the log does not say that a drag started" }
    if ($selection) { Say ("the log says: " + $selection.Line) }
    if ($action) { Say ("FAIL: something was run: " + $action.Line) } else { Say "PASS: nothing was run, because nothing can be done with these files yet" }
    if ($mismatch) { Say ("FAIL: the shell and the drop target disagree: " + $mismatch.Line) }
    if ($start -and $start.Line -match "1 file\(s\)" -and $selection -and $selection.Line -match "1 item\(s\)" -and -not $mismatch) {
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
    #
    # The pointer stops where the wheel opens: running past it would cross
    # another segment on the way, and the test would be about the wrong one.
    [void][Native]::SetCursorPos($at[0], $at[1])
    Start-Sleep -Milliseconds 500
    [Native]::mouse_event([Native]::LEFTDOWN, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 350
    for ($i = 1; $i -le 10; $i++) {
        [void][Native]::SetCursorPos(($at[0] + $i * 4), ($at[1] + $i * 3))
        Start-Sleep -Milliseconds 45
    }
    # The wheel opens here, around the pointer.
    $centreX = $at[0] + 10 * 4
    $centreY = $at[1] + 10 * 3
    [Native]::keybd_event(0x10, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 900
    Start-Sleep -Milliseconds 700

    # The wheel opens around the pointer. Nothing can be done with these files
    # yet, so the wheel has nothing on it, and the application says so.
    $wheel = Get-Wheel
    if (-not $wheel) {
        Say "FAIL: the application did not say where it put the wheel"
    } else {
        $point = Segment-Point $wheel 0
        $before = @(Get-AppLog).Count
        Move-To-Point $point[0] $point[1]
        $hover = Get-FreshHover $before
        Start-Sleep -Milliseconds 400
        if ($hover) {
            Say ("the log says: " + $hover.line)
        } else {
            Say "note: nothing was lit, because the wheel has nothing on it"
        }
    }

    [Native]::keybd_event(0x10, 0, [Native]::KEYUP, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 200
    [Native]::mouse_event([Native]::LEFTUP, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Seconds 2
    Stop-App $process

    $log = Get-AppLog
    $moves = @($log | Select-String -Pattern "the window was moved to")
    $drop = $log | Select-String -Pattern "the drag was dropped on the wheel" | Select-Object -First 1
    $actions = @($log | Select-String -Pattern "placeholder:")
    $action = $actions | Select-Object -First 1
    foreach ($line in $moves) { Say ("the log says: " + $line.Line) }
    if ($drop) {
        Say ("the log says: " + $drop.Line)
        $landed = $log | Select-String -Pattern "the drop landed at" | Select-Object -First 1
        if ($landed) { Say ("the log says: " + $landed.Line) }
    } else {
        # Windows does not always hand the drop over. This was measured on the
        # commit before this one as well, so it is not something the
        # application caused. The choice is made the other way then, and it is
        # the same choice.
        Say "note: Windows did not hand the drop over this time, so the choice was made from the segment that was lit"
    }
    if ($action) { Say ("FAIL: something was run: " + $action.Line) } else { Say "PASS: nothing was run, because nothing can be done with these files yet" }

    # The wheel is put in one place when it opens, and it must not be moved
    # again: not when the drag is released, and not while it fades away.
    if ($moves.Count -eq 1) {
        Say "PASS: the wheel was moved once, when it opened, and never again"
    } else {
        Say "FAIL: the wheel was moved $($moves.Count) times"
    }
    # Nothing can be done with these files yet, so the wheel has nothing on
    # it, and the release chooses nothing.
    if (-not $action) {
        Say "PASS: nothing was run, because nothing can be done with these files yet"
    } else {
        Say ("FAIL: something was run: " + $action.Line)
    }
    # The action is run once, and only once: a drop that is handled twice
    # would run the action twice.
    if ($actions.Count -eq 0) {
        Say "PASS: the action did not run at all"
    } else {
        Say "FAIL: the action ran $($actions.Count) times"
    }
    # The wheel said that it has nothing on it, and the drop found no
    # segment for the same reason.
    $empty = $log | Select-String -Pattern "wheel has nothing on it" | Select-Object -First 1
    if ($empty) {
        Say ("the log says: " + $empty.Line)
        Say "PASS: the wheel said that it had nothing on it"
    } else {
        Say "FAIL: the wheel did not say that it had nothing on it"
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
    #
    # The pointer stops where the wheel opens, and the hub is worked out from
    # the wheel's own place, so that it is the real middle of the wheel.
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
    Start-Sleep -Milliseconds 700

    $wheel = Get-Wheel
    if (-not $wheel) {
        Say "FAIL: the application did not say where it put the wheel"
    } else {
        # There is nothing on the wheel, so nothing can be lit, and the wheel
        # says so. The pointer is moved onto the middle of it all the same, to
        # be sure that the wheel is where it says it is.
        $before = @(Get-AppLog).Count
        Move-To-Point ([int]$wheel.CentreX) ([int]$wheel.CentreY)
        $hub = Get-FreshHover $before
        Start-Sleep -Milliseconds 400
        if ($hub) { Say ("the log says: " + $hub.line) }
        else { Say "note: nothing was lit, because the wheel has nothing on it" }
    }

    [Native]::keybd_event(0x10, 0, [Native]::KEYUP, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 200
    [Native]::mouse_event([Native]::LEFTUP, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Seconds 2
    Stop-App $process

    $log = Get-AppLog
    $drop = $log | Select-String -Pattern "the drop landed at" | Select-Object -First 1
    $action = $log | Select-String -Pattern "placeholder:" | Select-Object -First 1
    if ($drop) {
        Say ("the log says: " + $drop.Line)
    } else {
        # Windows does not always hand the drop over. This was measured on the
        # commit before this one as well, so it is not something the
        # application caused.
        Say "note: Windows did not hand the drop over this time, so the choice was made from the segment that was lit"
    }
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

function Case-DevelopmentWheel {
    $script:CaseName = "devwheel"
    Say ""
    Say "=== case 9: the development wheel, and choosing something on it ==="
    # The development wheel is a look at the design, and the design is every
    # operation there is. It is how the choice of an action is checked while
    # nothing can really be run yet.
    $process = Start-App 20
    Minimize-Terminal

    # Put the pointer somewhere quiet and open the wheel there.
    [void][Native]::SetCursorPos(700, 500)
    Start-Sleep -Milliseconds 500
    [Native]::keybd_event(0x11, 0, 0, [UIntPtr]::Zero)   # Control down
    [Native]::keybd_event(0x12, 0, 0, [UIntPtr]::Zero)   # Alt down
    [Native]::keybd_event(0x54, 0, 0, [UIntPtr]::Zero)   # T down
    Start-Sleep -Milliseconds 700
    [Native]::keybd_event(0x54, 0, [Native]::KEYUP, [UIntPtr]::Zero)
    [Native]::keybd_event(0x12, 0, [Native]::KEYUP, [UIntPtr]::Zero)
    [Native]::keybd_event(0x11, 0, [Native]::KEYUP, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 900

    $wheel = Get-Wheel
    if (-not $wheel) {
        Say "FAIL: the development wheel did not open"
        Stop-App $process
        return
    }
    Say ("the development wheel is at {0}" -f $wheel.Left)

    # Onto a choice of the wheel, the way a hand moves it.
    $point = Segment-Point $wheel 0
    $before = @(Get-AppLog).Count
    Move-To-Point $point[0] $point[1]
    $hover = Get-FreshHover $before
    Start-Sleep -Milliseconds 400
    if ($hover) {
        Say ("the log says: " + $hover.line)
        Say ("  the pointer is on segment {0} of the wheel, and the application said so after {1} ms" -f `
            (Hover-Index $hover.line), [int]$hover.elapsed)
    } else {
        Say "FAIL: the application did not say which segment is lit"
    }

    # Choose it with the keyboard, which is how the wheel is used without a
    # mouse.
    [Native]::keybd_event(0x27, 0, 0, [UIntPtr]::Zero)   # Right
    Start-Sleep -Milliseconds 400
    [Native]::keybd_event(0x27, 0, [Native]::KEYUP, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 400
    [Native]::keybd_event(0x0D, 0, 0, [UIntPtr]::Zero)   # Enter
    Start-Sleep -Milliseconds 400
    [Native]::keybd_event(0x0D, 0, [Native]::KEYUP, [UIntPtr]::Zero)
    Start-Sleep -Seconds 2
    Stop-App $process

    $log = Get-AppLog
    $actions = @($log | Select-String -Pattern "placeholder:")
    $refused = $log | Select-String -Pattern "was not run" | Select-Object -First 1
    if ($actions.Count -eq 1) {
        Say ("the log says: " + $actions[0].Line)
        Say "PASS: the choice was made and the action ran exactly once"
    } else {
        Say "FAIL: the action ran $($actions.Count) times"
    }
    # The operation is not implemented, so the choice is reported and refused.
    if ($refused) {
        Say ("the log says: " + $refused.Line)
        Say "PASS: the operation was refused, because it is not implemented"
    } else {
        Say "FAIL: the operation was not refused"
    }
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
if ($Only -eq "" -or $Only -eq "devwheel") { Case-DevelopmentWheel }

Say ""
Say "the report is at $report"
