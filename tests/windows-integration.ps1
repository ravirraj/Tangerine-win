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
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint processId);
    // The title is a real null, and not an empty string: a window that has a
    // title of its own is only found when the title is a null.
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowEx(IntPtr parent, IntPtr after, string cls, IntPtr title);
    [DllImport("user32.dll")] public static extern bool ScreenToClient(IntPtr hWnd, ref POINT lpPoint);
    [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr hWnd, uint Msg, IntPtr wParam, IntPtr lParam);
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
        Minimize-Window $terminal.MainWindowHandle
    }
}

# Puts a window out of the way, and remembers it.
#
# The windows that are put out of the way are given back afterwards, so that
# the person who is watching does not have to bring them back by hand.
#
# The window the tests are run from, and the windows of the programs that
# started it, are left alone: they are the window the person is watching,
# and putting it away takes the run away with it.
$script:PutAway = @()
$script:Ancestors = $null

function Get-AncestorProcessIds {
    if ($null -ne $script:Ancestors) { return $script:Ancestors }
    $ids = @{}
    $id = $PID
    for ($step = 0; $step -lt 12 -and $id -gt 0; $step++) {
        if ($ids.ContainsKey($id)) { break }
        $ids[$id] = $true
        $line = Get-CimInstance Win32_Process -Filter "ProcessId=$id" -ErrorAction SilentlyContinue
        if (-not $line) { break }
        $id = [int]$line.ParentProcessId
    }
    $script:Ancestors = $ids
    return $ids
}

function Minimize-Window($handle) {
    if (-not $handle -or $handle -eq [IntPtr]::Zero) { return }
    $handle = [IntPtr]$handle
    [uint32]$owner = 0
    [void][Native]::GetWindowThreadProcessId($handle, [ref]$owner)
    if ((Get-AncestorProcessIds).ContainsKey([int]$owner)) {
        # This window belongs to the program the tests are run from, or to
        # the program that started it. It is left as it is.
        return
    }
    if ($script:PutAway -contains $handle) { return }
    $script:PutAway += $handle
    [void][Native]::ShowWindow($handle, 6)
    Start-Sleep -Milliseconds 200
}

# Gives the windows that were put out of the way back.
function Restore-Windows {
    foreach ($handle in $script:PutAway) {
        [void][Native]::ShowWindow($handle, 9)
    }
    $script:PutAway = @()
    Start-Sleep -Milliseconds 300
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
# RadialGeometry in src/radial/geometry.rs and WheelProfile::design: the
# design's wheel measures 360 pixels across, and it is drawn smaller than
# that, with an inner radius of 60 and an outer radius of 168 in the
# design's own terms, cut into eight segments with the angles measured
# clockwise from the top. The radii are given as a part of the wheel's
# size, so a display that is scaled comes out right without anything being
# assumed about it.
$WheelSegments = 8
$WheelInnerRatio = 60.0 / 360.0
$WheelOuterRatio = 168.0 / 360.0

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
    $start = $log | Select-String -Pattern "drag \d+ started in the window" | Select-Object -First 1
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
    $start = $log | Select-String -Pattern "drag \d+ started in the window" | Select-Object -First 1
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
    $starts = @($log | Select-String -Pattern "drag \d+ started in the window")
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
    $start = $log | Select-String -Pattern "drag \d+ started in the window" | Select-Object -First 1
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
    $start = $log | Select-String -Pattern "drag \d+ started in the window" | Select-Object -First 1
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
    $action = $log | Select-String -Pattern "the operation .* was not run|the operation .* wrote" | Select-Object -First 1
    $cancelled = $log | Select-String -Pattern "let go of on the hub" | Select-Object -First 1
    if ($drop) {
        Say ("the log says: " + $drop.Line)
    } else {
        # Windows does not always hand the drop over. This was measured on the
        # commit before this one as well, so it is not something the
        # application caused.
        Say "note: Windows did not hand the drop over this time, so the choice was made from the segment that was lit"
    }
    if ($action) { Say ("FAIL: something was run: " + $action.Line) } else { Say "PASS: nothing was run" }
    # The hub is not a choice: the interaction is cancelled and the wheel goes
    # away, so that it is not in the way of the next drag.
    if ($cancelled) {
        Say ("the log says: " + $cancelled.Line)
        Say "PASS: the hub cancelled the interaction"
    } else {
        Say "FAIL: the hub did not cancel the interaction"
    }
}

function Case-ReleasedOutside {
    $script:CaseName = "outside"
    Say ""
    Say "=== case 11: a drag that is released outside the wheel ==="
    # A file that is let go of away from the wheel is not a choice: the
    # interaction is cancelled, nothing is run, and the wheel goes away.
    $folder = Join-Path $root "itest-outside"
    if (Test-Path $folder) { Remove-Item -Recurse -Force $folder }
    New-Item -ItemType Directory -Force -Path $folder | Out-Null
    Set-Content (Join-Path $folder "outside.txt") "hello"

    Get-Process explorer -ErrorAction SilentlyContinue | ForEach-Object { $_.CloseMainWindow() | Out-Null }
    Start-Sleep -Seconds 2
    Minimize-Terminal
    Start-Process explorer.exe -ArgumentList "/e,`"$folder`""
    Start-Sleep -Seconds 4

    $window = Get-ExplorerWindow "itest-outside*"
    if ($null -eq $window) { Say "FAIL: the Explorer window did not open"; return }
    $hwnd = [IntPtr]$window.Current.NativeWindowHandle
    [void][Native]::ShowWindow($hwnd, 3)
    [void][Native]::SetForegroundWindow($hwnd)
    Start-Sleep -Seconds 2
    Minimize-Terminal

    $at = Select-Item "itest-outside*" "outside.txt" $false
    Minimize-Terminal
    [void][Native]::SetForegroundWindow($hwnd)
    Start-Sleep -Milliseconds 500

    $process = Start-App 25
    Minimize-Terminal
    [void][Native]::SetForegroundWindow($hwnd)
    Start-Sleep -Milliseconds 500

    # The wheel opens around the pointer, and the file is then dragged away
    # from it and let go of there.
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

    # Away from the wheel: the pointer is moved well clear of it, so that the
    # drag leaves the wheel before the file is let go of.
    $wheel = Get-Wheel
    if (-not $wheel) {
        Say "FAIL: the application did not say where it put the wheel"
    } else {
        [void][Native]::SetCursorPos([int]($wheel.CentreX + 500), [int]($wheel.CentreY + 320))
        Start-Sleep -Milliseconds 700
    }

    [Native]::keybd_event(0x10, 0, [Native]::KEYUP, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 200
    [Native]::mouse_event([Native]::LEFTUP, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Seconds 2
    Stop-App $process

    $log = Get-AppLog
    $action = $log | Select-String -Pattern "the operation .* was not run|the operation .* wrote" | Select-Object -First 1
    $cancelled = $log | Select-String -Pattern "ended outside the wheel|stayed outside the wheel" | Select-Object -First 1
    if ($action) { Say ("FAIL: something was run: " + $action.Line) } else { Say "PASS: nothing was run" }
    if ($cancelled) {
        Say ("the log says: " + $cancelled.Line)
        Say "PASS: the drag was cancelled"
    } else {
        Say "FAIL: the drag was not cancelled"
    }
}

function Get-DesktopIconList {
    # The list the desktop's icons are in.
    #
    # The desktop is a shell window of its own, and its icons live in a list
    # view inside it. The application looks for the list in the same way.
    $desktop = [System.Windows.Automation.AutomationElement]::RootElement
    $condition = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ClassNameProperty, "Progman")
    $windows = $desktop.FindAll([System.Windows.Automation.TreeScope]::Children, $condition)
    foreach ($w in $windows) {
        $hwnd = [IntPtr]$w.Current.NativeWindowHandle
        $view = [Native]::FindWindowEx($hwnd, [IntPtr]::Zero, "SHELLDLL_DefView", [IntPtr]::Zero)
        if ($view -ne [IntPtr]::Zero) {
            $list = [Native]::FindWindowEx($view, [IntPtr]::Zero, "SysListView32", [IntPtr]::Zero)
            if ($list -ne [IntPtr]::Zero) { return $list }
        }
    }
    return [IntPtr]::Zero
}

function Get-DesktopSelection {
    # The index of the first selected icon on the desktop, or -1 when nothing
    # is selected.
    #
    # This is how the application tells an icon from the empty desktop: the
    # shell selects the icon that is pressed, and it clears the selection when
    # the empty desktop is pressed.
    param($list)
    # The search starts before the first item, so that the first selected
    # item is found whatever it is.
    return [Native]::SendMessage($list, 0x100C, [IntPtr](-1), [IntPtr]2).ToInt32()
}

function Minimize-OtherWindows {
    # The desktop has to be visible for a drag to start on it.
    #
    # Every window that is not one of the shell's own is put out of the way,
    # so that the icons can be reached. The windows are given back at the end
    # of the case.
    $shell = @("Progman", "WorkerW", "Shell_TrayWnd", "CabinetWClass", "ExploreWClass")
    $root = [System.Windows.Automation.AutomationElement]::RootElement
    $windows = $root.FindAll([System.Windows.Automation.TreeScope]::Children,
        [System.Windows.Automation.Condition]::TrueCondition)
    foreach ($w in $windows) {
        if ($shell -notcontains $w.Current.ClassName) {
            Minimize-Window ([IntPtr]$w.Current.NativeWindowHandle)
        }
    }
    Start-Sleep -Milliseconds 600
}

function Find-DesktopIcon {
    # Finds the point on the desktop's icon with the given name.
    #
    # The icons are in a grid in the top left corner of the desktop. A point
    # is tried by clicking it: a click on an icon selects it, and the shell
    # says which icon it was. The shell's list of the virtual desktop is in
    # the same order as the icons, so the name of the item at the place the
    # selection gives is the name of the icon.
    #
    # The special items — This PC, the Recycle Bin and the like — are not
    # clicked: clicking one of them opens it, and the desktop is in a
    # different order afterwards.
    param($list, [string]$wanted, [int]$first)
    $shell = New-Object -ComObject Shell.Application
    $virtual = $shell.Namespace(0).Items()
    for ($row = 0; $row -lt 12; $row++) {
        for ($column = 0; $column -lt 6; $column++) {
            $x = 15 + $column * 80
            $y = 15 + $row * 85
            [void][Native]::SetCursorPos($x, $y)
            Start-Sleep -Milliseconds 220
            [Native]::mouse_event([Native]::LEFTDOWN, 0, 0, 0, [UIntPtr]::Zero)
            Start-Sleep -Milliseconds 110
            [Native]::mouse_event([Native]::LEFTUP, 0, 0, 0, [UIntPtr]::Zero)
            Start-Sleep -Milliseconds 320
            $selected = Get-DesktopSelection $list
            if ($selected -lt 0) { continue }
            if ($selected -lt $first) { continue }
            if ($selected -lt $virtual.Count -and $virtual.Item($selected).Name -eq $wanted) {
                return @{ X = $x; Y = $y }
            }
        }
    }
    return $null
}

function Get-DesktopFileIndex {
    # The place of a file in the desktop's own list of items.
    #
    # The shell's list of the *virtual* desktop holds the same items as the
    # icons on the screen, in the same order: the special items first, and
    # then the files of the desktop's folder. So the place of a file in that
    # list is the place of its icon.
    param([string]$name)
    $shell = New-Object -ComObject Shell.Application
    $items = $shell.Namespace(0).Items()
    for ($i = 0; $i -lt $items.Count; $i++) {
        if ($items.Item($i).Name -eq $name) { return $i }
    }
    return -1
}

function Get-DesktopFirstFileIndex {
    # The place of the first file of the desktop's folder.
    #
    # The items before it are the special ones — This PC, the Recycle Bin and
    # the like — and clicking on one of them opens it, which puts the desktop
    # in a different order. The test stays away from them.
    $shell = New-Object -ComObject Shell.Application
    $files = $shell.Namespace([Environment]::GetFolderPath("Desktop")).Items()
    if ($files.Count -eq 0) { return 0 }
    $first = $files.Item(0).Name
    $virtual = $shell.Namespace(0).Items()
    for ($i = 0; $i -lt $virtual.Count; $i++) {
        if ($virtual.Item($i).Name -eq $first) { return $i }
    }
    return 0
}

function Get-UnusedExtension {
    # An extension that no file on the desktop has.
    #
    # The application does not write file names in its log, so the kind of the
    # file is what says which file it was. A kind that nothing else on the
    # desktop has therefore names one file and no other.
    $taken = @{}
    $shell = New-Object -ComObject Shell.Application
    foreach ($folder in @([Environment]::GetFolderPath("Desktop"), "C:\Users\Public\Desktop")) {
        if (-not (Test-Path $folder)) { continue }
        Get-ChildItem $folder -File -ErrorAction SilentlyContinue | ForEach-Object {
            if ($_.Extension) { $taken[$_.Extension.TrimStart(".").ToLower()] = $true }
        }
    }
    foreach ($extension in @("csv", "xlsx", "pptx", "epub", "zip", "yaml", "toml", "md")) {
        if (-not $taken.ContainsKey($extension)) { return $extension }
    }
    return "txt"
}

function Find-EmptyDesktop {
    # Finds a point on the desktop that is not on an icon.
    #
    # The click clears the selection, so a point is empty when the selection
    # is empty after it has been clicked.
    param($list)
    for ($row = 0; $row -lt 5; $row++) {
        for ($column = 0; $column -lt 4; $column++) {
            $x = 1400 + $column * 90
            $y = 200 + $row * 90
            [void][Native]::SetCursorPos($x, $y)
            Start-Sleep -Milliseconds 250
            [Native]::mouse_event([Native]::LEFTDOWN, 0, 0, 0, [UIntPtr]::Zero)
            Start-Sleep -Milliseconds 120
            [Native]::mouse_event([Native]::LEFTUP, 0, 0, 0, [UIntPtr]::Zero)
            Start-Sleep -Milliseconds 350
            if ((Get-DesktopSelection $list) -lt 0) { return @{ X = $x; Y = $y } }
        }
    }
    return $null
}

function Case-DesktopDrag {
    $script:CaseName = "desktop"
    Say ""
    Say "=== case 5: a drag that starts on the desktop ==="
    # The desktop is a source of files in its own right. A drag that starts
    # on one of its icons must open the wheel, and an ordinary click on the
    # empty desktop must not.
    Get-Process explorer -ErrorAction SilentlyContinue | ForEach-Object { $_.CloseMainWindow() | Out-Null }
    Start-Sleep -Seconds 3

    # The desktop has to be visible, or the icons cannot be reached.
    Minimize-OtherWindows

    # A picture of its own on the desktop, in a format that nothing else
    # there has, so that the application says which file was dragged.
    $made = New-DesktopPicture
    $stem = $made.Stem
    $file = $made.Path

    $list = Get-DesktopIconList
    if ($list -eq [IntPtr]::Zero) { Say "FAIL: the desktop's list of icons was not found"; return }

    $empty = Find-EmptyDesktop $list
    if (-not $empty) { Say "FAIL: no empty place was found on the desktop"; return }
    Say ("an empty place on the desktop is at ({0}, {1})" -f $empty.X, $empty.Y)

    $process = Start-App 240
    Minimize-Terminal
    Start-Sleep -Milliseconds 600

    # The icon of the picture that was made. It is found by dragging it: the
    # application says what format the drag carries, and the format of the
    # picture is one nothing else on the desktop has.
    $found = Invoke-DesktopDragOfFormat "bmp"
    if (-not $found) {
        Say "FAIL: the icon of the picture was not found on the desktop"
        Stop-App $process
        return
    }
    $icon = @{ X = $found.X; Y = $found.Y }
    Say ("the icon of the picture is at ({0}, {1})" -f $icon.X, $icon.Y)
    Dismiss-Wheel

    # First the empty desktop: a click and a drag there is not a drag of a
    # file, so the wheel must not open.
    $before = @(Get-AppLog).Count
    [void][Native]::SetCursorPos($empty.X, $empty.Y)
    Start-Sleep -Milliseconds 500
    [Native]::mouse_event([Native]::LEFTDOWN, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 300
    for ($i = 1; $i -le 12; $i++) {
        [void][Native]::SetCursorPos(($empty.X + $i * 10), ($empty.Y + $i * 8))
        Start-Sleep -Milliseconds 45
    }
    [Native]::keybd_event(0x10, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 700
    [Native]::keybd_event(0x10, 0, [Native]::KEYUP, [UIntPtr]::Zero)
    [Native]::mouse_event([Native]::LEFTUP, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Seconds 2

    # Only the lines of this gesture are looked at: the lines of an earlier
    # gesture say nothing about this one.
    $all = @(Get-AppLog)
    $log = @()
    if ($all.Count -gt $before) { $log = $all[$before..($all.Count - 1)] }
    $rejected = $log | Select-String -Pattern "carries no file" | Select-Object -First 1
    $started = $log | Select-String -Pattern "drag \d+ started in the" | Select-Object -First 1
    $opened = $log | Select-String -Pattern "the wheel is opening" | Select-Object -First 1
    if ($rejected) { Say ("the log says: " + $rejected.Line) } else { Say "FAIL: the drag of the empty desktop was not refused" }
    if ($started) { Say ("FAIL: a drag was started anyway: " + $started.Line) } else { Say "PASS: no drag was started" }
    if ($opened) { Say ("FAIL: the wheel opened anyway: " + $opened.Line) } else { Say "PASS: the wheel did not open" }

    # And now the icon: a drag of a file on the desktop must open the wheel.
    #
    # The pointer is moved once the wheel is up, the way a user moves onto a
    # choice. The drop target is registered when the wheel is drawn, and the
    # shell tells it what the drag carries when the pointer is over it, so the
    # move is what brings the file list in.
    $lines = Invoke-DesktopDrag $icon.X $icon.Y $false
    Stop-App $process

    $started = $lines | Select-String -Pattern "drag \d+ started in the" | Select-Object -First 1
    $opened = $lines | Select-String -Pattern "the wheel is opening" | Select-Object -First 1
    if ($started) { Say ("the log says: " + $started.Line) } else { Say "FAIL: the log does not say that a drag started" }
    if ($opened) { Say ("the log says: " + $opened.Line) } else { Say "FAIL: the wheel did not open" }
    if ($started -and $opened) { Say "PASS: a drag of a desktop file opened the wheel" }
    else { Say "FAIL: a drag of a desktop file did not open the wheel" }
    # The wheel knows what is being dragged. The log does not name the file,
    # so the format of the file is what can be checked: the picture is the
    # only file of that format on the desktop.
    $selection = $lines | Select-String -Pattern "the selection holds" | Select-Object -Last 1
    if ($selection) { Say ("the log says: " + $selection.Line) }
    if ($selection -and $selection.Line -match "1 item\(s\) of 1 kind\(s\) \(image\), in the format\(s\) bmp") {
        Say "PASS: the wheel knows what is being dragged"
    } else {
        Say "FAIL: the wheel does not know what is being dragged"
    }

    Remove-Item -Force $file -ErrorAction SilentlyContinue
    # The conversion may have left a file of its own next to the picture.
    Get-ChildItem ([Environment]::GetFolderPath("Desktop")) -Filter "$stem.*" -File -ErrorAction SilentlyContinue |
        ForEach-Object { Remove-Item -Force $_.FullName -ErrorAction SilentlyContinue }
}

# Drags a place on the desktop and lets go of it.
#
# When $choose is true, the pointer is moved around the wheel until it is
# over the choice that makes a JPEG, and the file is let go of there. The
# application says which choice the pointer is over, so the choice is found
# from what the application says and not from an assumption about the shape
# of the wheel.
#
# It gives back the lines the application wrote while the drag was going.
function Invoke-DesktopDrag($x, $y, [bool]$choose) {
    $before = @(Get-AppLog).Count
    [void][Native]::SetCursorPos($x, $y)
    Start-Sleep -Milliseconds 400
    [Native]::mouse_event([Native]::LEFTDOWN, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 300
    for ($i = 1; $i -le 8; $i++) {
        [void][Native]::SetCursorPos(($x + $i * 14), ($y + $i * 10))
        Start-Sleep -Milliseconds 40
    }
    [Native]::keybd_event(0x10, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 800
    $wheel = Get-Wheel
    if (-not $wheel) {
        Note "  (the application did not say where it put the wheel)"
    } elseif ($choose) {
        # The pointer is walked around the wheel until it is over the choice
        # that makes a JPEG. The wheel is only a drop target a moment after
        # it is drawn, so the walk is given time: the pointer has to move
        # after that, or the drag is never told what it carries.
        $radius = $wheel.Size * ($WheelInnerRatio + $WheelOuterRatio) / 2
        for ($step = 0; $step -lt 24; $step++) {
            $rad = ($step * 15) * [Math]::PI / 180
            [void][Native]::SetCursorPos([int]($wheel.CentreX + $radius * [Math]::Sin($rad)),
                                         [int]($wheel.CentreY - $radius * [Math]::Cos($rad)))
            Start-Sleep -Milliseconds 150
            # Only the lines of this drag are looked at: a line from an
            # earlier drag says nothing about where the pointer is now.
            $lines = @(Get-AppLog)
            $over = $lines[$before..($lines.Count - 1)] |
                Select-String -Pattern "the pointer is over (\w+)" | Select-Object -Last 1
            if ($over -and $over.Matches[0].Groups[1].Value -eq "JPEG") { break }
        }
    } else {
        # The pointer is left on the hub, so that nothing is chosen and no
        # file of the person using the machine is changed.
        #
        # The pointer is moved about inside the hub for a while. Windows
        # looks for the window under the pointer when the pointer moves, and
        # the wheel is only a drop target a moment after it is drawn: the
        # pointer has to move after that, or the drag is never told what it
        # carries.
        for ($step = 0; $step -lt 4; $step++) {
            Move-To-Point ([int]($wheel.CentreX + 12)) ([int]($wheel.CentreY + 8))
            Start-Sleep -Milliseconds 200
            Move-To-Point ([int]$wheel.CentreX) ([int]$wheel.CentreY)
            Start-Sleep -Milliseconds 200
        }
        Start-Sleep -Milliseconds 300
    }
    [Native]::keybd_event(0x10, 0, [Native]::KEYUP, [UIntPtr]::Zero)
    [Native]::mouse_event([Native]::LEFTUP, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Seconds 2
    $lines = @(Get-AppLog)
    if ($lines.Count -le $before) { return @() }
    return $lines[$before..($lines.Count - 1)]
}

# Takes the wheel away.
#
# A drag that chooses nothing leaves the wheel up, and the wheel would then
# be in the way of the drag after it: the pointer would be over the wheel and
# not over the desktop. Escape is how a person takes it away.
function Dismiss-Wheel {
    $before = @(Get-AppLog).Count
    [Native]::keybd_event(0x1B, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 200
    [Native]::keybd_event(0x1B, 0, [Native]::KEYUP, [UIntPtr]::Zero)
    # The wheel is taken away over a moment, and the desktop is only clear
    # when the window is hidden.
    $deadline = (Get-Date).AddSeconds(6)
    while ((Get-Date) -lt $deadline) {
        $lines = @(Get-AppLog)
        if ($lines.Count -gt $before) {
            $hidden = $lines[$before..($lines.Count - 1)] |
                Where-Object { $_ -match "the overlay was hidden" }
            if ($hidden) { Start-Sleep -Milliseconds 400; return }
        }
        Start-Sleep -Milliseconds 100
    }
}

# The places on the desktop where an icon may be.
#
# The icons are in a grid in the top left corner of the desktop. The special
# items — This PC, the Recycle Bin and the like — come first and are not
# dragged: one of them opens a window, and the desktop is in a different
# order afterwards.
function Get-DesktopCells {
    $cells = @()
    for ($column = 1; $column -lt 6; $column++) {
        for ($row = 0; $row -lt 12; $row++) {
            if ($column -eq 1 -and $row -lt 3) { continue }
            $cells += @{ X = (15 + $column * 80); Y = (15 + $row * 85) }
        }
    }
    return $cells
}

# Drags the places on the desktop until one of them carries a file of the
# given format, and gives back the drag that carried it.
#
# The icons on the desktop are not in the order of the shell's list of the
# desktop, and they move when a file is added to the desktop, so the icon of
# a file is found by dragging the icons: the application says what format
# the drag carries, and a format that nothing else on the desktop has
# therefore names one file and no other.
#
# The pointer is left on the hub, so that nothing is chosen and no file of
# the person using the machine is changed.
function Invoke-DesktopDragOfFormat($format) {
    foreach ($cell in (Get-DesktopCells)) {
        $lines = Invoke-DesktopDrag $cell.X $cell.Y $false
        $carried = $lines | Select-String -Pattern "in the format\(s\) (\w+)" | Select-Object -First 1
        if ($carried -and $carried.Matches[0].Groups[1].Value -eq $format) {
            Note ("  the icon at ({0}, {1}) carries the {2} file" -f $cell.X, $cell.Y, $format)
            return @{ X = $cell.X; Y = $cell.Y; Lines = $lines }
        }
        Dismiss-Wheel
    }
    return $null
}

# Says what the application made of a drag, and whether the wheel had
# something to offer.
#
# The file list of a drag that started on the desktop comes from the drop
# target, and it only arrives once the pointer is over the wheel. The wheel
# must offer the operations of the file that is really being dragged, and
# not the empty wheel that it opens with.
#
# $expectOperations says what the file is: a picture has operations, and a
# file that is not a picture has none, and that is said plainly.
function Check-DesktopDrag($lines, $label, [bool]$expectOperations = $true) {
    $carried = $lines | Select-String -Pattern "the drop target reports" | Select-Object -First 1
    $selection = $lines | Select-String -Pattern "the selection holds" | Select-Object -Last 1
    $offered = $lines | Select-String -Pattern "operations of the convert wheel can be used" | Select-Object -Last 1
    $ok = $true
    if ($carried) {
        Note ("  " + $label + ": the log says: " + $carried.Line)
    } else {
        Note ("FAIL: " + $label + ": the drop target did not report what the drag carried")
        $ok = $false
    }
    if ($selection) {
        Note ("  " + $label + ": the log says: " + $selection.Line)
    } else {
        Note ("FAIL: " + $label + ": the application did not say what was selected")
        $ok = $false
    }
    if (-not $expectOperations) { return $ok }
    $count = 0
    if ($offered -and $offered.Line -match "(\d+) of the \d+ operations") { $count = [int]$Matches[1] }
    if ($count -lt 1) {
        Note ("FAIL: " + $label + ": the wheel had nothing to offer")
        if ($offered) { Note ("  the log says: " + $offered.Line) }
        $ok = $false
    } else {
        Note ("PASS: " + $label + ": the wheel knows what is being dragged")
    }
    return $ok
}

# Makes a picture on the desktop in a format that nothing else there has,
# and gives back its name and the places of the icons that carry a file.
#
# The format is what names the file: the application says what format the
# drag carries, and a format that nothing else on the desktop has therefore
# names one file and no other.
function New-DesktopPicture {
    # What an earlier run of a test may have left behind is taken away
    # first: a file of the test's own making would be found by the search
    # for the picture, and the test would be about the wrong file.
    Get-ChildItem ([Environment]::GetFolderPath("Desktop")) -File -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -like "0000-desktop-test-*" -or $_.Name -like "0000-desktop-repeat-*" } |
        ForEach-Object { Remove-Item -Force $_.FullName -ErrorAction SilentlyContinue }
    $stem = "0000-desktop-test-$([DateTime]::Now.Ticks)"
    $path = Join-Path ([Environment]::GetFolderPath("Desktop")) "$stem.bmp"
    $picture = New-Object System.Drawing.Bitmap 64, 64
    for ($x = 0; $x -lt 64; $x++) {
        for ($y = 0; $y -lt 64; $y++) { $picture.SetPixel($x, $y, [System.Drawing.Color]::Red) }
    }
    $picture.Save($path, [System.Drawing.Imaging.ImageFormat]::Bmp)
    $picture.Dispose()
    Start-Sleep -Seconds 4
    return @{ Stem = $stem; Path = $path }
}

function Case-DesktopRepeat {
    $script:CaseName = "desktoprepeat"
    Say ""
    Say "=== case 10: the same desktop file, dragged again ==="
    # A file may be dragged as often as the person wants. Every drag is a
    # drag of its own, and the wheel must offer the operations of the file
    # that is being dragged now.
    Get-Process explorer -ErrorAction SilentlyContinue | ForEach-Object { $_.CloseMainWindow() | Out-Null }
    Start-Sleep -Seconds 3
    Minimize-OtherWindows

    $desktop = [Environment]::GetFolderPath("Desktop")
    # A picture in a format that nothing else on the desktop has, so that
    # the application says which file was dragged.
    $made = New-DesktopPicture
    $stem = $made.Stem

    $process = Start-App 300
    Minimize-Terminal
    Start-Sleep -Milliseconds 600

    # The icon of the picture. It is found by dragging, and the application
    # says what format the drag carries.
    $found = Invoke-DesktopDragOfFormat "bmp"
    if (-not $found) {
        Say "FAIL: the icon of the picture was not found on the desktop"
        Stop-App $process
        return
    }
    $icon = @{ X = $found.X; Y = $found.Y }
    Dismiss-Wheel

    # The first drag, and the conversion. The wheel goes away when the work
    # is done, so the drag after it is a drag of the desktop and not of the
    # wheel.
    $lines = Invoke-DesktopDrag $icon.X $icon.Y $true
    $ok = Check-DesktopDrag $lines "the first drag"
    $made = @(Get-ChildItem $desktop -Filter "$stem.*" -File -ErrorAction SilentlyContinue)
    if ($made.Count -ge 2) {
        Note "PASS: the conversion made a file beside the picture"
    } else {
        Note "FAIL: the conversion made no file"
        $ok = $false
    }

    # The same file again, right after the conversion. This is the case that
    # used to come up with an empty wheel.
    #
    # The conversion put a file on the desktop, and the desktop moves its
    # icons when that happens, so the icon is looked for again.
    $found = Invoke-DesktopDragOfFormat "bmp"
    if (-not $found) {
        Say "FAIL: the icon of the picture was not found on the desktop"
        Stop-App $process
        return
    }
    $lines = $found.Lines
    if (-not (Check-DesktopDrag $lines "the second drag, the same file")) { $ok = $false }
    Dismiss-Wheel

    # A different file. What it is does not matter: what matters is that the
    # drag carries something else, and that the drag after it carries its
    # own file again.
    $other = Invoke-DesktopDragOfFormat "png"
    if ($other) { $lines = $other.Lines } else { $lines = @() }
    if (-not (Check-DesktopDrag $lines "the third drag, another file" $false)) { $ok = $false }
    Dismiss-Wheel

    # And the first file once more.
    $found = Invoke-DesktopDragOfFormat "bmp"
    if (-not $found) {
        Say "FAIL: the icon of the picture was not found on the desktop"
        Stop-App $process
        return
    }
    $lines = $found.Lines
    if (-not (Check-DesktopDrag $lines "the fourth drag, the first file again")) { $ok = $false }
    Dismiss-Wheel

    Stop-App $process
    if ($ok) { Say "PASS: every drag of the desktop file knew what it was carrying" }

    Get-ChildItem $desktop -Filter "$stem.*" -File -ErrorAction SilentlyContinue |
        ForEach-Object { Remove-Item -Force $_.FullName -ErrorAction SilentlyContinue }
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
    $actions = @($log | Select-String -Pattern "the operation .* was not run|the operation .* wrote")
    $refused = $log | Select-String -Pattern "was not run" | Select-Object -First 1
    if ($actions.Count -eq 1) {
        Say ("the log says: " + $actions[0].Line)
        Say "PASS: the choice was made and the operation was tried once"
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

# Runs a case, and gives the windows that were put out of the way back.
#
# The person who is watching the tests gets their windows back after every
# case, so that a case does not leave the machine in a state nobody expects.
function Run-Case($case) {
    & $case
    Restore-Windows
}

if ($Only -eq "" -or $Only -eq "one") { Run-Case { Case-OneFile } }
if ($Only -eq "" -or $Only -eq "many") { Run-Case { Case-MultipleFiles } }
if ($Only -eq "" -or $Only -eq "consecutive") { Run-Case { Case-Consecutive } }
if ($Only -eq "" -or $Only -eq "cancel") { Run-Case { Case-Cancelled } }
if ($Only -eq "" -or $Only -eq "unselected") { Run-Case { Case-Unselected } }
if ($Only -eq "" -or $Only -eq "stays") { Run-Case { Case-WheelStays } }
if ($Only -eq "" -or $Only -eq "hub") { Run-Case { Case-DropOnHub } }
if ($Only -eq "" -or $Only -eq "outside") { Run-Case { Case-ReleasedOutside } }
if ($Only -eq "" -or $Only -eq "desktop") { Run-Case { Case-DesktopDrag } }
if ($Only -eq "" -or $Only -eq "desktoprepeat") { Run-Case { Case-DesktopRepeat } }
if ($Only -eq "" -or $Only -eq "devwheel") { Run-Case { Case-DevelopmentWheel } }

Restore-Windows

Say ""
Say "the report is at $report"
