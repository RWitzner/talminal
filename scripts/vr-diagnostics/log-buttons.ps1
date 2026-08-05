# Hvad sender Quest-controllerne som Windows-input i remote desktop / passthrough?
# Logger ALLE fem musetaster + et udvalg af taster paa fysisk niveau
# (GetAsyncKeyState), samt hvilket vindue der har forgrunden.
#
# Brug: powershell -ExecutionPolicy Bypass -File log-buttons.ps1 -Seconds 30
#
# Tryk EEN knap ad gangen og HOLD den ca. 2 sekunder, med en pause imellem.
# Foreslaaet raekkefoelge: trigger, grip, A, B, thumbstick-klik, menu.

param([int]$Seconds = 30)

Add-Type -Namespace VR -Name Btn -MemberDefinition @'
[DllImport("user32.dll")] public static extern short GetAsyncKeyState(int k);
[DllImport("user32.dll")] public static extern System.IntPtr GetForegroundWindow();
[DllImport("user32.dll")] public static extern int GetWindowThreadProcessId(System.IntPtr h, out int pid);
'@

# Raekkefoelgen er bevidst: musetasterne foerst, for det er dem der betyder noget her.
$WATCH = [ordered]@{
  'Mouse1 (venstre)'  = 0x01
  'Mouse2 (hoejre)'   = 0x02
  'Mouse3 (midter)'   = 0x04
  'Mouse4 (XBUTTON1)' = 0x05
  'Mouse5 (XBUTTON2)' = 0x06
  'Escape'            = 0x1B
  'Space'             = 0x20
  'Enter'             = 0x0D
  'Tab'               = 0x09
  'F8'                = 0x77
}

$names = @{}
function Get-FgName([IntPtr]$h) {
  if (-not $names.ContainsKey($h)) {
    [int]$procId = 0
    [void][VR.Btn]::GetWindowThreadProcessId($h, [ref]$procId)
    $p = Get-Process -Id $procId -ErrorAction SilentlyContinue
    $names[$h] = if ($p) { $p.ProcessName } else { "pid$procId" }
  }
  return $names[$h]
}

Write-Host ""
Write-Host "Logger i $Seconds sekunder." -ForegroundColor Cyan
Write-Host "Tryk EEN knap ad gangen og HOLD ca. 2 sekunder. Pause imellem." -ForegroundColor Cyan
Write-Host "Foreslaaet: trigger -> grip -> A -> B -> thumbstick-klik -> menu" -ForegroundColor Cyan
Write-Host ""

$state = @{}; $best = @{}; $run = @{}; $fgAt = @{}
foreach ($k in $WATCH.Keys) { $state[$k] = $false; $best[$k] = 0; $run[$k] = 0 }

$end = (Get-Date).AddSeconds($Seconds)
while ((Get-Date) -lt $end) {
  $fg = Get-FgName ([VR.Btn]::GetForegroundWindow())
  foreach ($k in $WATCH.Keys) {
    $down = [bool]([VR.Btn]::GetAsyncKeyState($WATCH[$k]) -band 0x8000)
    if ($down) {
      $run[$k]++
      if ($run[$k] -gt $best[$k]) { $best[$k] = $run[$k]; $fgAt[$k] = $fg }
      if (-not $state[$k]) {
        $state[$k] = $true
        Write-Host ("  NED  {0,-18} (forgrund: {1})" -f $k, $fg) -ForegroundColor Green
      }
    } else {
      if ($state[$k]) {
        $ms = $run[$k] * 10
        Write-Host ("  OP   {0,-18} holdt ~{1} ms" -f $k, $ms) -ForegroundColor DarkGray
        $state[$k] = $false
      }
      $run[$k] = 0
    }
  }
  Start-Sleep -Milliseconds 10
}

Write-Host ""
Write-Host "=== OPSAMLING ===" -ForegroundColor Cyan
$seen = $false
foreach ($k in $WATCH.Keys) {
  if ($best[$k] -eq 0) { continue }
  $seen = $true
  $ms = $best[$k] * 10
  $verdict = if ($best[$k] -ge 50) { "HOLD ($ms ms) - brugbar som push-to-talk" } else { "kort tryk ($ms ms) - muligvis kun et klik" }
  Write-Host ("  {0,-18} {1}; forgrund: {2}" -f $k, $verdict, $fgAt[$k])
}
if (-not $seen) {
  Write-Host "  INGEN af de overvaagede taster blev set nede." -ForegroundColor Red
  Write-Host "  Controller-input naar altsaa ikke Windows som fysisk muse-/tastetilstand"
  Write-Host "  i denne tilstand. Da er en flade i appen selv den rigtige vej."
} else {
  Write-Host ""
  Write-Host "Er 'Mouse4 (XBUTTON1)' paa listen med HOLD, er du faerdig: din PTT staar" -ForegroundColor Green
  Write-Host "allerede paa Mouse4, og der skal ingen kode til." -ForegroundColor Green
  Write-Host "Er kun Mouse1/Mouse2 paa listen, duer de IKKE som PTT - de bruges til at klikke." -ForegroundColor Yellow
}
