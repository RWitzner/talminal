# M4-BRO: en hold-flade i browseren der injicerer et AEGTE Mouse4 (XBUTTON1)
# paa Windows, saa Talminals GetAsyncKeyState-poller kan se det.
#
# En browser kan IKKE selv lave OS-input. Derfor denne lille lokale proces:
#   pointerdown paa fladen -> POST /down -> mouse_event(MOUSEEVENTF_XDOWN, XBUTTON1)
#   pointerup   paa fladen -> POST /up   -> mouse_event(MOUSEEVENTF_XUP,   XBUTTON1)
#
# Brug: powershell -ExecutionPolicy Bypass -File m4-bridge.ps1
# Aabn derefter http://127.0.0.1:17345/ - stop med Ctrl+C.
#
# SIKKERHED: binder KUN til 127.0.0.1. Ingen ekstern adgang.
# SIKRING:   watchdog i baade side og server slipper knappen hvis noget haenger.

param([int]$Port = 17345, [int]$MaxHoldMs = 10000)

Add-Type -Namespace M4 -Name Native -MemberDefinition @'
[DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint d, System.UIntPtr e);
[DllImport("user32.dll")] public static extern short GetAsyncKeyState(int k);
[DllImport("user32.dll")] public static extern System.IntPtr GetForegroundWindow();
[DllImport("user32.dll")] public static extern int GetWindowThreadProcessId(System.IntPtr h, out int pid);
'@

$XDOWN = 0x0080; $XUP = 0x0100; $XBUTTON1 = 1; $VK_XBUTTON1 = 0x05
$script:isDown = $false
$script:downAt = $null

function Get-FgName {
  $h = [M4.Native]::GetForegroundWindow()
  [int]$procId = 0
  [void][M4.Native]::GetWindowThreadProcessId($h, [ref]$procId)
  $p = Get-Process -Id $procId -ErrorAction SilentlyContinue
  if ($p) { return $p.ProcessName } else { return "pid$procId" }
}

function Push-M4Down {
  if ($script:isDown) { return "allerede nede" }
  [M4.Native]::mouse_event($XDOWN, 0, 0, $XBUTTON1, [UIntPtr]::Zero)
  $script:isDown = $true
  $script:downAt = Get-Date
  $seen = [bool]([M4.Native]::GetAsyncKeyState($VK_XBUTTON1) -band 0x8000)
  $fg = Get-FgName
  Write-Host ("  NED  M4 injiceret - GetAsyncKeyState ser den: {0} - forgrund: {1}" -f $seen, $fg) -ForegroundColor Green
  if ($fg -notmatch 'talminal') {
    Write-Host "       (fokus-gaten i Talminal vil kassere denne kant - se noten i terminalen)" -ForegroundColor Yellow
  }
  return "ned (async=$seen, forgrund=$fg)"
}

function Push-M4Up([string]$why = "up") {
  if (-not $script:isDown) { return "var ikke nede" }
  [M4.Native]::mouse_event($XUP, 0, 0, $XBUTTON1, [UIntPtr]::Zero)
  $script:isDown = $false
  $ms = [int]((Get-Date) - $script:downAt).TotalMilliseconds
  Write-Host ("  OP   M4 sluppet efter {0} ms ({1})" -f $ms, $why) -ForegroundColor DarkGray
  return "op efter $ms ms"
}

$html = @'
<!doctype html>
<meta charset="utf-8">
<title>M4-bro - hold fladen</title>
<style>
  :root { color-scheme: dark; }
  body { margin:0; background:#02060c; color:#d8e2ef; padding:24px;
         font:20px/1.4 "Segoe UI", system-ui, sans-serif; }
  h1 { font-size:22px; margin:0 0 14px; font-weight:600; }
  #pad { height:300px; border-radius:16px; display:grid; place-items:center;
         background:#0d1b2a; border:3px solid #1f3a52; user-select:none;
         touch-action:none; font-size:34px; font-weight:600; cursor:pointer; }
  #pad.down { background:#14532d; border-color:#22c55e; }
  #log { margin-top:16px; max-height:240px; overflow:auto;
         font:15px/1.5 "Cascadia Mono", Consolas, monospace; white-space:pre; }
  .ok { color:#4ade80; } .warn { color:#fbbf24; } .err { color:#f87171; }
</style>

<h1>Hold fladen nede = Mouse4 holdes nede paa Windows</h1>
<div id="pad">HOLD MIG</div>
<div id="log"></div>

<script>
  const pad = document.getElementById("pad");
  const log = document.getElementById("log");
  const out = (m, c) => {
    const d = document.createElement("div");
    if (c) d.className = c;
    d.textContent = new Date().toISOString().slice(11,23) + "  " + m;
    log.prepend(d);
  };

  let held = false, downAt = 0, guard = null;

  async function send(path) {
    try {
      const r = await fetch(path, { method: "POST" });
      out(path + " -> " + (await r.text()), "ok");
    } catch (e) {
      out(path + " FEJLEDE: " + e.message, "err");
    }
  }

  function down(e) {
    if (held) return;
    held = true; downAt = performance.now();
    pad.setPointerCapture(e.pointerId);
    pad.classList.add("down");
    out("pointerdown type=" + e.pointerType + " id=" + e.pointerId);
    send("/down");
    // Klient-watchdog: slip ALTID inden for MAXHOLD, ogsaa hvis up gaar tabt.
    guard = setTimeout(() => { out("klient-watchdog slipper", "warn"); up("watchdog"); }, MAXHOLD);
  }

  function up(why) {
    if (!held) return;
    held = false;
    clearTimeout(guard); guard = null;
    pad.classList.remove("down");
    out("slip efter " + Math.round(performance.now() - downAt) + " ms (" + why + ")");
    send("/up");
  }

  pad.addEventListener("pointerdown", down);
  pad.addEventListener("pointerup", () => up("pointerup"));
  pad.addEventListener("pointercancel", () => up("pointercancel"));
  // Netop fordi injektionen kan flytte fokus: slip ogsaa paa blur og skjult side.
  window.addEventListener("blur", () => up("blur"));
  document.addEventListener("visibilitychange", () => { if (document.hidden) up("hidden"); });
  window.addEventListener("pagehide", () => up("pagehide"));

  out("klar - hold fladen nede. Serveren logger i terminalen.");
</script>
'@ -replace 'MAXHOLD', $MaxHoldMs

$listener = New-Object System.Net.HttpListener
$prefix = "http://127.0.0.1:$Port/"
$listener.Prefixes.Add($prefix)
try { $listener.Start() } catch {
  Write-Host "Kunne ikke lytte paa $prefix - er porten i brug?" -ForegroundColor Red
  Write-Host $_.Exception.Message
  exit 1
}

Write-Host ""
Write-Host "M4-bro lytter paa $prefix" -ForegroundColor Cyan
Write-Host "Aabn adressen i browseren og hold fladen nede. Ctrl+C stopper." -ForegroundColor Cyan
Write-Host ""
Write-Host "VIGTIGT om fokus-gaten: naar du holder fladen i BROWSEREN, er browseren" -ForegroundColor Yellow
Write-Host "forgrundsvindue - ikke Talminal. Talminals poller er fail-closed og" -ForegroundColor Yellow
Write-Host "kasserer derfor kanten. Injektionen er stadig AEGTE, hvilket log-m4.ps1" -ForegroundColor Yellow
Write-Host "kan bevise. Det er praecis maalingen der viser om VR-tilstanden (aaben" -ForegroundColor Yellow
Write-Host "gate + betinget blur-cancel) er noedvendig." -ForegroundColor Yellow
Write-Host ""

try {
  while ($listener.IsListening) {
    $ctx = $listener.GetContext()
    $path = $ctx.Request.Url.AbsolutePath
    $body = ""
    $type = "text/plain; charset=utf-8"

    switch ($path) {
      "/"      { $body = $html; $type = "text/html; charset=utf-8" }
      "/down"  { $body = Push-M4Down }
      "/up"    { $body = Push-M4Up "pointerup" }
      "/state" {
        $seen = [bool]([M4.Native]::GetAsyncKeyState($VK_XBUTTON1) -band 0x8000)
        $body = "isDown=$($script:isDown) async=$seen forgrund=$(Get-FgName)"
      }
      default  { $ctx.Response.StatusCode = 404; $body = "ukendt: $path" }
    }

    # Server-watchdog: har knappen staaet nede for laenge, slip den uanset hvad.
    if ($script:isDown -and ((Get-Date) - $script:downAt).TotalMilliseconds -gt ($MaxHoldMs + 2000)) {
      [void](Push-M4Up "server-watchdog")
    }

    $bytes = [Text.Encoding]::UTF8.GetBytes($body)
    $ctx.Response.ContentType = $type
    $ctx.Response.ContentLength64 = $bytes.Length
    $ctx.Response.OutputStream.Write($bytes, 0, $bytes.Length)
    $ctx.Response.Close()
  }
} finally {
  # Uden dette kunne knappen blive haengende nede systemvidt efter Ctrl+C.
  [void](Push-M4Up "server lukker")
  $listener.Stop(); $listener.Close()
  Write-Host ""
  Write-Host "M4-bro stoppet, og knappen er sluppet." -ForegroundColor Cyan
}
