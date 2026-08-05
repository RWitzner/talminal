# Trin 5 - verificer at controller-knappen giver et HOLDT tryk, og at canvas
# ejer forgrunden i samme oejeblik. Begge skal vaere sande samtidig; intet andet
# betyder noget.
#
# Brug:  powershell -ExecutionPolicy Bypass -File log-m4.ps1 -Seconds 20
# Tag headsettet paa, hold controller-knappen i 3 sekunder, kom tilbage.

param([int]$Seconds = 20)

Add-Type -Namespace VR -Name Log -MemberDefinition @'
[DllImport("user32.dll")] public static extern short GetAsyncKeyState(int k);
[DllImport("user32.dll")] public static extern System.IntPtr GetForegroundWindow();
[DllImport("user32.dll")] public static extern int GetWindowThreadProcessId(System.IntPtr h, out int pid);
'@

$VK = @{ 'M4' = 0x05; 'M5' = 0x06; 'F8' = 0x77; 'F9' = 0x78 }
$samples = [System.Collections.Generic.List[object]]::new()
$names = @{}

Write-Host "Logger i $Seconds sekunder. Hold controller-knappen nu." -ForegroundColor Cyan
$end = (Get-Date).AddSeconds($Seconds)
while ((Get-Date) -lt $end) {
  $h = [VR.Log]::GetForegroundWindow()
  if (-not $names.ContainsKey($h)) {
    [int]$procId = 0; [void][VR.Log]::GetWindowThreadProcessId($h, [ref]$procId)
    $p = Get-Process -Id $procId -ErrorAction SilentlyContinue
    $names[$h] = if ($p) { $p.ProcessName } else { "pid$procId" }
  }
  $row = [ordered]@{ t = (Get-Date -Format 'HH:mm:ss.fff'); fg = $names[$h] }
  foreach ($k in $VK.Keys) { $row[$k] = [bool]([VR.Log]::GetAsyncKeyState($VK[$k]) -band 0x8000) }
  $samples.Add([pscustomobject]$row)
  Start-Sleep -Milliseconds 10
}

$csv = Join-Path $PSScriptRoot 'm4-log.csv'
$samples | Export-Csv $csv -NoTypeInformation -Encoding UTF8

Write-Host ""
Write-Host "$($samples.Count) stikproever -> $csv"
Write-Host "Forgrundsvinduer set: $((($samples.fg | Sort-Object -Unique) -join ', '))"
Write-Host ""

foreach ($k in @('M4','M5','F8','F9')) {
  $best = 0; $run = 0; $fgDuringBest = ''
  foreach ($s in $samples) {
    if ($s.$k) { $run++; if ($run -gt $best) { $best = $run; $fgDuringBest = $s.fg } } else { $run = 0 }
  }
  if ($best -eq 0) { continue }
  $ms = $best * 10
  $verdict = if ($best -ge 20) { "HOLD bekraeftet (~${ms} ms)" } else { "kun et TAP (~${ms} ms) - noget er sat op som toggle/hotkey i stedet for en hold-action" }
  Write-Host ("{0}: {1}; forgrund under trykket = {2}" -f $k, $verdict, $fgDuringBest)
  if ($fgDuringBest -notmatch 'talminal') {
    Write-Host "   -> Tasten naaede frem, men canvas ejede IKKE forgrunden. Fixet ligger i din egen kode:" -ForegroundColor Yellow
    Write-Host "      aabn fokus-gaten (wake_hotkey.rs) OG goer blur-cancel betinget (App.tsx:1246)." -ForegroundColor Yellow
  }
}

if (-not ($samples | Where-Object { $_.M4 -or $_.M5 -or $_.F8 -or $_.F9 })) {
  Write-Host "Ingen af tasterne blev nogensinde set nede." -ForegroundColor Red
  Write-Host "  Tjek i denne raekkefoelge: (1) Talminal koerer IKKE som administrator (UIPI),"
  Write-Host "  (2) begge bindingssteder i Desktop+ (Global OG Active Controller Buttons),"
  Write-Host "  (3) SteamVR-dashboardet er lukket, (4) laserpointeren er ikke aktiv."
}
