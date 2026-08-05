# Skelner mellem tre verdener naar en controller peger paa skaermen:
#   1) VD roerer slet ikke Windows-input      -> markoer staar stille, ingen knapper
#   2) VD flytter markoeren, men uden knapper -> markoer bevaeger sig, nul knapper
#   3) VD sender aegte SendInput-museklik     -> markoer bevaeger sig OG knapper ses nede
#
# Brug: powershell -ExecutionPolicy Bypass -File log-pointer.ps1 -Seconds 20
#
# Bevaeg controlleren rundt HELE tiden, og tryk knapper undervejs.

param([int]$Seconds = 20)

Add-Type -Namespace VR -Name Ptr -MemberDefinition @'
[DllImport("user32.dll")] public static extern short GetAsyncKeyState(int k);
[DllImport("user32.dll")] public static extern bool GetCursorPos(out System.Drawing.Point p);
[DllImport("user32.dll")] public static extern System.IntPtr GetForegroundWindow();
[DllImport("user32.dll")] public static extern int GetWindowThreadProcessId(System.IntPtr h, out int pid);
'@ -ReferencedAssemblies System.Drawing

$BTN = [ordered]@{
  'Mouse1 (venstre)'  = 0x01
  'Mouse2 (hoejre)'   = 0x02
  'Mouse3 (midter)'   = 0x04
  'Mouse4 (XBUTTON1)' = 0x05
  'Mouse5 (XBUTTON2)' = 0x06
}

$down    = @{}   # knap -> tidspunkt for tryk
$holds   = @{}   # knap -> liste af holdvarigheder i ms
foreach ($k in $BTN.Keys) { $down[$k] = $null; $holds[$k] = New-Object System.Collections.ArrayList }

$moves   = 0
$last    = New-Object System.Drawing.Point
[void][VR.Ptr]::GetCursorPos([ref]$last)
$start   = $last
$minX = $last.X; $maxX = $last.X; $minY = $last.Y; $maxY = $last.Y

Write-Host ""
Write-Host "  ... 3"; Start-Sleep -Milliseconds 700
Write-Host "  ... 2"; Start-Sleep -Milliseconds 700
Write-Host "  ... 1"; Start-Sleep -Milliseconds 700
Write-Host ""
Write-Host "BEVAEG CONTROLLEREN NU - og tryk knapper undervejs. $Seconds sekunder."
Write-Host ""

$t0 = [Diagnostics.Stopwatch]::StartNew()
while ($t0.Elapsed.TotalSeconds -lt $Seconds) {
  $p = New-Object System.Drawing.Point
  if ([VR.Ptr]::GetCursorPos([ref]$p)) {
    if ($p.X -ne $last.X -or $p.Y -ne $last.Y) {
      $moves++
      if ($p.X -lt $minX) { $minX = $p.X }
      if ($p.X -gt $maxX) { $maxX = $p.X }
      if ($p.Y -lt $minY) { $minY = $p.Y }
      if ($p.Y -gt $maxY) { $maxY = $p.Y }
      $last = $p
    }
  }
  foreach ($k in $BTN.Keys) {
    $isDown = ([VR.Ptr]::GetAsyncKeyState($BTN[$k]) -band 0x8000) -ne 0
    if ($isDown -and $null -eq $down[$k]) {
      $down[$k] = $t0.Elapsed.TotalMilliseconds
      Write-Host ("  {0,6:N1}s  NED  {1}" -f $t0.Elapsed.TotalSeconds, $k)
    } elseif (-not $isDown -and $null -ne $down[$k]) {
      $ms = [int]($t0.Elapsed.TotalMilliseconds - $down[$k])
      [void]$holds[$k].Add($ms)
      $down[$k] = $null
      Write-Host ("  {0,6:N1}s  OP   {1}  ({2} ms)" -f $t0.Elapsed.TotalSeconds, $k, $ms)
    }
  }
  Start-Sleep -Milliseconds 30
}
$t0.Stop()

Write-Host ""
Write-Host "=== MARKOER ==="
Write-Host ("  bevaegelser registreret : {0}" -f $moves)
Write-Host ("  start                   : {0},{1}" -f $start.X, $start.Y)
Write-Host ("  slut                    : {0},{1}" -f $last.X, $last.Y)
Write-Host ("  omraade daekket         : X {0}..{1}  Y {2}..{3}" -f $minX, $maxX, $minY, $maxY)

Write-Host ""
Write-Host "=== KNAPPER ==="
$any = $false
foreach ($k in $BTN.Keys) {
  if ($holds[$k].Count -gt 0) {
    $any = $true
    $avg = [int](($holds[$k] | Measure-Object -Average).Average)
    Write-Host ("  {0,-20} {1} tryk, gennemsnit {2} ms, alle: {3}" -f $k, $holds[$k].Count, $avg, ($holds[$k] -join ', '))
  }
  if ($null -ne $down[$k]) {
    $any = $true
    Write-Host ("  {0,-20} stadig NEDE da maalingen sluttede" -f $k)
  }
}
if (-not $any) { Write-Host "  ingen musetaster set nede" }

Write-Host ""
Write-Host "=== KONKLUSION ==="
if ($moves -eq 0 -and -not $any) {
  Write-Host "  VERDEN 1: controlleren roerer slet ikke Windows-input."
  Write-Host "  Hverken markoer eller knapper. En flade i appen selv er vejen."
} elseif ($moves -gt 0 -and -not $any) {
  Write-Host "  VERDEN 2: markoeren flyttes, men der kommer INGEN aegte musetaster."
  Write-Host "  Klik injiceres da udenom den fysiske tastetilstand (som Metas touch)."
  Write-Host "  GetAsyncKeyState-hotkeys er strukturelt blinde. Brug en flade i appen."
} elseif ($any) {
  # Et hold er kun et hold hvis varigheden overlever. Et syntetiseret klik
  # ankommer ogsaa som "aegte" fysisk tilstand, men kollapser til <200 ms
  # uanset hvor laenge knappen holdes - og saa er push-to-talk umuligt.
  $longest = 0
  foreach ($k in $BTN.Keys) {
    foreach ($ms in $holds[$k]) { if ($ms -gt $longest) { $longest = $ms } }
    if ($null -ne $down[$k]) { $longest = [int]$Seconds * 1000 }  # stadig nede = aegte hold
  }
  if ($longest -ge 1000) {
    Write-Host ("  VERDEN 3: AEGTE hold naar Windows. Laengste: {0} ms." -f $longest)
    Write-Host "  En hold-binding (fx Mouse4) kan drives direkte fra controlleren."
  } else {
    Write-Host ("  VERDEN 2b: knapper ankommer som fysisk tilstand, men som KLIK." -f $longest)
    Write-Host ("  Laengste hold var kun {0} ms - varigheden overlever ikke turen." -f $longest)
    Write-Host "  Push-to-talk ved hold er udelukket. En TOGGLE passer derimod"
    Write-Host "  perfekt til det transporten faktisk leverer: rene, korte klik."
  }
}
