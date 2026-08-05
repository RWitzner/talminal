# Trin 3 - bevis M4-kaeden UDEN VR i ligningen.
# Injicerer et aegte XBUTTON1 (Mouse4) hold paa 3 sekunder via mouse_event,
# som fodrer samme input-koe som SendInput og dermed saetter den asynkrone
# tastetilstand Talminals poller (wake_hotkey.rs, GetAsyncKeyState) laeser.
#
# Brug:  powershell -ExecutionPolicy Bypass -File test-m4.ps1
# Klik ind i Talminals canvas mens den taeller ned.

Add-Type -Namespace VR -Name Native -MemberDefinition @'
[DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint d, System.UIntPtr e);
[DllImport("user32.dll")] public static extern System.IntPtr GetForegroundWindow();
[DllImport("user32.dll")] public static extern int GetWindowThreadProcessId(System.IntPtr h, out int pid);
[DllImport("user32.dll")] public static extern short GetAsyncKeyState(int k);
'@

$XDOWN = 0x0080; $XUP = 0x0100; $XBUTTON1 = 1; $VK_XBUTTON1 = 0x05

function Get-ForegroundName {
  $h = [VR.Native]::GetForegroundWindow()
  [int]$procId = 0; [void][VR.Native]::GetWindowThreadProcessId($h, [ref]$procId)
  $p = Get-Process -Id $procId -ErrorAction SilentlyContinue
  if ($p) { return "$($p.ProcessName) (pid $procId)" } else { return "ukendt (pid $procId)" }
}

if (-not (Get-Process -Name 'talminal*' -ErrorAction SilentlyContinue)) {
  Write-Host "ADVARSEL: Talminal koerer ikke. Start den foerst." -ForegroundColor Yellow
}

Write-Host ""
Write-Host "Klik ind i Talminals canvas NU." -ForegroundColor Cyan
3..1 | ForEach-Object { Write-Host "  $_..." ; Start-Sleep -Milliseconds 1000 }

$fg = Get-ForegroundName
Write-Host ""
Write-Host "Forgrundsvindue ved tryk: $fg"
if ($fg -notmatch 'talminal') {
  Write-Host "  -> Fokus-gaten VIL afvise trykket. Testen siger derfor intet om polleren." -ForegroundColor Yellow
}

Write-Host "M4 NED - holder 3 sekunder. Tal nu." -ForegroundColor Green
[VR.Native]::mouse_event($XDOWN, 0, 0, $XBUTTON1, [UIntPtr]::Zero)

# Bekraeft undervejs at tilstanden faktisk STAAR nede (samme kilde som appen laeser)
$held = 0
1..30 | ForEach-Object {
  if ([VR.Native]::GetAsyncKeyState($VK_XBUTTON1) -band 0x8000) { $held++ }
  Start-Sleep -Milliseconds 100
}

[VR.Native]::mouse_event($XUP, 0, 0, $XBUTTON1, [UIntPtr]::Zero)
Write-Host "M4 OP." -ForegroundColor Green
Write-Host ""
Write-Host "GetAsyncKeyState saa tasten nede i $held/30 stikproever."
if ($held -ge 28) {
  Write-Host "  -> Holdet blev leveret korrekt paa fysisk niveau." -ForegroundColor Green
  Write-Host "     Gik Talminal i lyttetilstand, er HELE kaeden bevist: injektion -> poller -> gate -> mikrofon -> STT."
  Write-Host "     Gik den IKKE, ligger fejlen efter polleren: fokus-gate, mikrofon eller STT."
} else {
  Write-Host "  -> Holdet blev IKKE leveret. Noget spiser inputtet (anden hook, UIPI, eller eleveret forgrundsvindue)." -ForegroundColor Red
}
