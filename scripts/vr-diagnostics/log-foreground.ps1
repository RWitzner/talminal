# Hvad ER forgrundsvinduet mens du holder triggeren i gamepad-tilstand?
#
# Fokus-gaten i wake_hotkey.rs sammenligner GetForegroundWindow() med Talminals
# egen HWND hvert 5. ms, og braender holdet hvis komboen fuldendes ufokuseret.
# Den er TAVS for bare bindinger, saa fejler den, er der ingen log. Derfor
# maales den her, hvor vi stadig kan se den.
#
# Brug: powershell -ExecutionPolicy Bypass -File log-foreground.ps1 -Seconds 25

param([int]$Seconds = 25)

Add-Type -Namespace VR -Name Fg -MemberDefinition @'
[DllImport("user32.dll")] public static extern System.IntPtr GetForegroundWindow();
[DllImport("user32.dll")] public static extern int GetWindowThreadProcessId(System.IntPtr h, out int pid);
[DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(System.IntPtr h, System.Text.StringBuilder s, int n);
'@

function Get-Fg {
  $h = [VR.Fg]::GetForegroundWindow()
  [int]$procId = 0
  [void][VR.Fg]::GetWindowThreadProcessId($h, [ref]$procId)
  $p = Get-Process -Id $procId -ErrorAction SilentlyContinue
  $sb = New-Object System.Text.StringBuilder 512
  [void][VR.Fg]::GetWindowTextW($h, $sb, $sb.Capacity)
  [pscustomobject]@{
    Handle = $h
    Name   = if ($p) { $p.ProcessName } else { "pid$procId" }
    Title  = $sb.ToString()
  }
}

Write-Host ""
Write-Host "Talminal-processer lige nu:"
$tal = Get-Process | Where-Object { $_.ProcessName -like '*talminal*' }
if ($tal) {
  $tal | ForEach-Object { Write-Host ("  {0}  pid {1}  hwnd {2}  '{3}'" -f $_.ProcessName, $_.Id, $_.MainWindowHandle, $_.MainWindowTitle) }
} else {
  Write-Host "  INGEN - aabn Talminal foerst, ellers maaler vi ingenting."
}

Write-Host ""
Write-Host "  ... 3"; Start-Sleep -Milliseconds 700
Write-Host "  ... 2"; Start-Sleep -Milliseconds 700
Write-Host "  ... 1"; Start-Sleep -Milliseconds 700
Write-Host ""
Write-Host "NU: slaa gamepad-tilstand til, og HOLD saa triggeren i 3 sekunder. $Seconds sekunder."
Write-Host ""

$samples = @{}
$total = 0
$last = $null
$t0 = [Diagnostics.Stopwatch]::StartNew()
while ($t0.Elapsed.TotalSeconds -lt $Seconds) {
  $fg = Get-Fg
  $key = "$($fg.Name)|$($fg.Title)"
  if (-not $samples.ContainsKey($key)) { $samples[$key] = 0 }
  $samples[$key]++
  $total++
  if ($key -ne $last) {
    Write-Host ("  {0,6:N1}s  -> {1}  '{2}'" -f $t0.Elapsed.TotalSeconds, $fg.Name, $fg.Title)
    $last = $key
  }
  Start-Sleep -Milliseconds 100
}
$t0.Stop()

Write-Host ""
Write-Host "=== FORDELING ==="
$samples.GetEnumerator() | Sort-Object Value -Descending | ForEach-Object {
  $pct = [math]::Round(100.0 * $_.Value / $total, 1)
  $parts = $_.Key -split '\|', 2
  Write-Host ("  {0,5:N1}%  {1}  '{2}'" -f $pct, $parts[0], $parts[1])
}

Write-Host ""
Write-Host "=== KONKLUSION ==="
$talTime = 0
foreach ($k in $samples.Keys) { if ($k -like 'talminal*') { $talTime += $samples[$k] } }
$talPct = if ($total -gt 0) { [math]::Round(100.0 * $talTime / $total, 1) } else { 0 }
if ($talPct -ge 99) {
  Write-Host "  Talminal havde forgrunden HELE tiden ($talPct%). Fokus-gaten er tilfreds."
} elseif ($talPct -gt 0) {
  Write-Host "  Talminal havde forgrunden $talPct% af tiden - noget stjal den undervejs."
  Write-Host "  Sker det MENS triggeren holdes, braendes holdet og PTT doer TAVST."
} else {
  Write-Host "  Talminal havde ALDRIG forgrunden. Fokus-gaten ville afvise hvert tryk,"
  Write-Host "  uden en eneste log-linje (bar binding = SuppressedUnfocusedQuiet)."
}
