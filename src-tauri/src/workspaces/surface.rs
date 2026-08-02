//! Vinduets synlighed bag én seam.
//!
//! `conceal`/`reveal` er de eneste steder der rører synligheden. Slår WebView2's
//! baggrunds-throttling til på skjulte vinduer, kan implementationen skiftes til
//! at parkere vinduet uden for skærmen uden at ændre kalderne.

use super::geometry::{self, GeometryWriter, Rect, WindowPlacement};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use tauri::{Emitter, PhysicalPosition, PhysicalSize};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, GetForegroundWindow, IsZoomed, SetForegroundWindow,
};

pub trait WindowSurface: Send + Sync {
    /// Et ack må kun skrives efter et vellykket reveal.
    fn reveal(&self) -> Result<(), String>;
    fn conceal(&self);
    fn is_visible(&self) -> bool;
}

/// Test-dobbelt med kald-log.
pub struct FakeSurface {
    visible: AtomicBool,
    reveals: AtomicU32,
    conceals: AtomicU32,
    fail_next: AtomicBool,
}

impl FakeSurface {
    pub fn skjult() -> Self {
        Self::med(false)
    }

    pub fn synlig() -> Self {
        Self::med(true)
    }

    fn med(visible: bool) -> Self {
        Self {
            visible: AtomicBool::new(visible),
            reveals: AtomicU32::new(0),
            conceals: AtomicU32::new(0),
            fail_next: AtomicBool::new(false),
        }
    }

    pub fn reveal_kald(&self) -> u32 {
        self.reveals.load(Ordering::SeqCst)
    }

    pub fn conceal_kald(&self) -> u32 {
        self.conceals.load(Ordering::SeqCst)
    }

    pub fn fejl_ved_naeste_reveal(&self) {
        self.fail_next.store(true, Ordering::SeqCst);
    }
}

impl WindowSurface for FakeSurface {
    fn reveal(&self) -> Result<(), String> {
        self.reveals.fetch_add(1, Ordering::SeqCst);
        if self.fail_next.swap(false, Ordering::SeqCst) {
            return Err("fake: reveal fejlede".into());
        }
        self.visible.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn conceal(&self) {
        self.conceals.fetch_add(1, Ordering::SeqCst);
        self.visible.store(false, Ordering::SeqCst);
    }

    fn is_visible(&self) -> bool {
        self.visible.load(Ordering::SeqCst)
    }
}

/// Tauri-implementationen. `show()`/`hide()` er v1-vejen.
pub struct TauriSurface {
    window: tauri::WebviewWindow,
    geometry: Arc<GeometryWriter>,
}

impl TauriSurface {
    pub fn new(window: tauri::WebviewWindow, geometry: Arc<GeometryWriter>) -> Self {
        Self { window, geometry }
    }

    /// Vinduets FAKTISKE outer-geometri — ikke den der skal gemmes.
    ///
    /// Bevidst adskilt fra [`current_placement`], som bevarer den forrige
    /// normal-rect for et maksimeret vindue. Her skal vi bruge det raa svar fra
    /// Windows for at kunne afgoere om vinduet allerede staar rigtigt.
    ///
    /// **Maksimeringen laeses med `IsZoomed`, IKKE `WebviewWindow::is_maximized()`.**
    /// Sidstnaevnte returnerer tao's CACHEDE flag
    /// (`tao/src/platform_impl/windows/window.rs:613`:
    /// `window_flags.contains(WindowFlags::MAXIMIZED)`), og den cache skrives ud
    /// fra `WM_SIZE`s wParam (`event_loop.rs:1245`:
    /// `wparam.0 == SIZE_MAXIMIZED`) — altsaa fra BESKEDER, ikke fra vinduets
    /// tilstand. Baade `conceal` og `reveal` gaar gennem
    /// `WindowState::set_window_flags`, som ubetinget kalder
    /// `ShowWindow(SW_MAXIMIZE)` naar MAXIMIZED er sat (`window_state.rs:377` —
    /// grenen er `diff.contains(..) || new.contains(..)`, saa den fyrer OGSAA
    /// uden diff). Cachen kan derfor staa uenig med `IsZoomed`, og saa
    /// sammenlignede vi mod en paastand i stedet for mod virkeligheden — hvorefter
    /// geometrien blev anvendt hver gang alligevel, og hele fixet var
    /// virkningsloest. Rect'en er derimod i orden: `outer_position`/`outer_size`
    /// er `GetWindowRect`, som svarer korrekt ogsaa for et SKJULT vindue, og det
    /// er netop den tilstand `reveal` kalder fra.
    ///
    /// `None` naar bare én af aflaesningerne fejler — kalderen anvender da
    /// geometrien (se `geometry::placement_already_applied`s fail-safe).
    fn actual_geometry(&self) -> Option<(Rect, bool)> {
        let hwnd = self.window.hwnd().ok()?;
        let maximized = unsafe { IsZoomed(hwnd.0 as _) } != 0;
        let position = self.window.outer_position().ok()?;
        // INNER — det er den `set_size` saetter. Med `outer_size` sammenlignede vi
        // et tal mod noget der aldrig kunne blive lig det, og grenen var doed i
        // normal tilstand. Se `geometry::Rect`s doc.
        let size = self.window.inner_size().ok()?;
        Some((
            Rect {
                x: position.x,
                y: position.y,
                w: size.width as i32,
                h: size.height as i32,
            },
            maximized,
        ))
    }
}

impl WindowSurface for TauriSurface {
    fn reveal(&self) -> Result<(), String> {
        if let Some(placement) = geometry::read(&crate::project::global_base()) {
            let rect = geometry::clamp_to_work_area(placement.normal, &monitors(&self.window));
            // BETINGET. `apply_placement` er fire geometri-aendringer, og
            // geometrien er delt og global — den er i det almindelige skift
            // allerede korrekt. Se `geometry::placement_already_applied` for hvad
            // den ubetingede anvendelse kostede paa skaermen ved hvert skift.
            if !geometry::placement_already_applied(
                rect,
                placement.maximized,
                self.actual_geometry(),
            ) {
                apply_placement(&self.window, rect, placement.maximized)?;
            }
        }
        self.window.show().map_err(|e| format!("show: {e}"))?;
        self.geometry.suspend(false);
        let hwnd = self.window.hwnd().map_err(|e| format!("hwnd: {e}"))?;
        if !take_foreground(hwnd.0 as isize) {
            return Err("kunne ikke tage forgrunden".into());
        }
        let _ = self.window.emit("workspace-revealed", ());
        Ok(())
    }

    fn conceal(&self) {
        self.geometry.suspend(true);
        let _ = self.window.hide();
    }

    fn is_visible(&self) -> bool {
        self.window.is_visible().unwrap_or(false)
    }
}

/// Overdrager forgrundsretten fra den synlige proces til target-processen.
pub fn allow_foreground_for(pid: u32) {
    unsafe {
        AllowSetForegroundWindow(pid);
    }
}

/// Forsøger bounded at tage forgrunden og verificerer resultatet.
pub fn take_foreground(hwnd: isize) -> bool {
    for _ in 0..5 {
        unsafe {
            SetForegroundWindow(hwnd as _);
        }
        if unsafe { GetForegroundWindow() } as isize == hwnd {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    false
}

pub fn current_placement(window: &tauri::WebviewWindow) -> Option<WindowPlacement> {
    let maximized = window.is_maximized().unwrap_or(false);
    if maximized {
        let previous = geometry::read(&crate::project::global_base())?;
        return Some(WindowPlacement {
            normal: previous.normal,
            maximized: true,
        });
    }

    let position = window.outer_position().ok()?;
    // OUTER position + INNER size — samme par som `apply_placement` saetter.
    // Se `geometry::Rect`s doc.
    let size = window.inner_size().ok()?;
    Some(WindowPlacement {
        normal: Rect {
            x: position.x,
            y: position.y,
            w: size.width as i32,
            h: size.height as i32,
        },
        maximized: false,
    })
}

fn monitors(window: &tauri::WebviewWindow) -> Vec<Rect> {
    window
        .available_monitors()
        .unwrap_or_default()
        .into_iter()
        .map(|monitor| {
            let area = monitor.work_area();
            Rect {
                x: area.position.x,
                y: area.position.y,
                w: area.size.width as i32,
                h: area.size.height as i32,
            }
        })
        .collect()
}

fn apply_placement(
    window: &tauri::WebviewWindow,
    rect: Rect,
    maximized: bool,
) -> Result<(), String> {
    window
        .unmaximize()
        .map_err(|error| format!("unmaximize: {error}"))?;
    window
        .set_position(PhysicalPosition::new(rect.x, rect.y))
        .map_err(|error| format!("position: {error}"))?;
    window
        .set_size(PhysicalSize::new(
            rect.w.max(1) as u32,
            rect.h.max(1) as u32,
        ))
        .map_err(|error| format!("size: {error}"))?;
    if maximized {
        window
            .maximize()
            .map_err(|error| format!("maximize: {error}"))?;
    }
    Ok(())
}
