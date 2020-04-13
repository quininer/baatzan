use std::sync::Arc;
use parking_lot::Mutex;


pub struct ArcOption<T>(Mutex<Option<Arc<T>>>);

impl<T> ArcOption<T> {
    pub fn none() -> ArcOption<T> {
        ArcOption(Mutex::new(None))
    }

    pub fn get(&self) -> Option<Arc<T>> {
        self.0.lock().clone()
    }

    pub fn set(&self, t: Option<Arc<T>>) {
        *self.0.lock() = t;
    }

    pub fn take(&self) -> Option<Arc<T>> {
        self.0.lock().take()
    }
}
