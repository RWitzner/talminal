//! Samarbejdspolitikken set fra traad-siden (spec §4.3).

use std::sync::{Arc, Mutex, OnceLock};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcceptsFromView {
    Nobody,
    HumanOnly,
    List(Vec<String>),
    Any,
}

impl AcceptsFromView {
    pub fn allows(&self, sender: &str) -> bool {
        match self {
            AcceptsFromView::Any => true,
            AcceptsFromView::Nobody | AcceptsFromView::HumanOnly => false,
            AcceptsFromView::List(list) => list.iter().any(|c| c == sender),
        }
    }
}

pub trait Port: Send + Sync + 'static {
    fn read(&self, card: &str) -> AcceptsFromView;
    fn pair(&self, a: &str, b: &str) -> Result<(), String>;
    fn revoke(&self, a: &str, b: &str);
}

fn port() -> &'static Mutex<Option<Arc<dyn Port>>> {
    static P: OnceLock<Mutex<Option<Arc<dyn Port>>>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(None))
}

pub fn set_port(p: Arc<dyn Port>) {
    *port().lock().unwrap_or_else(|e| e.into_inner()) = Some(p);
}

pub fn is_wired() -> bool {
    port().lock().unwrap_or_else(|e| e.into_inner()).is_some()
}

fn current() -> Option<Arc<dyn Port>> {
    port().lock().unwrap_or_else(|e| e.into_inner()).clone()
}

pub fn read(card: &str) -> AcceptsFromView {
    match current() {
        Some(p) => p.read(card),
        None => AcceptsFromView::Nobody,
    }
}

pub fn pair(a: &str, b: &str) -> Result<(), String> {
    match current() {
        Some(p) => p.pair(a, b),
        None => Err("policy port is not wired".to_string()),
    }
}

pub fn revoke(a: &str, b: &str) {
    if let Some(p) = current() {
        p.revoke(a, b);
    }
}

#[cfg(feature = "test-seams")]
pub fn reset_for_test() {
    *port().lock().unwrap_or_else(|e| e.into_inner()) = None;
}

#[cfg(feature = "test-seams")]
pub fn set_fixed_for_test(view: AcceptsFromView) {
    struct Fixed(AcceptsFromView);
    impl Port for Fixed {
        fn read(&self, _card: &str) -> AcceptsFromView {
            self.0.clone()
        }
        fn pair(&self, _a: &str, _b: &str) -> Result<(), String> {
            Ok(())
        }
        fn revoke(&self, _a: &str, _b: &str) {}
    }
    set_port(Arc::new(Fixed(view)));
}
