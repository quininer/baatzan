use std::sync::Arc;


pub trait ThreadLocal<T> {
    fn with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&T) -> R;
}

pub trait Storage<T> {
    type Key: Ord;

    fn get(&self, key: &Self::Key) -> Option<Arc<T>>;
    fn remove(&self, key: &Self::Key) -> Option<Arc<T>>;
}

pub trait Insertable<T>: Storage<T> {
    fn insert(&self, key: Self::Key, val: T) -> Option<Arc<T>>;
}

pub trait Allocable<T>: Storage<T> {
    fn alloc(&self, val: T) -> Self::Key;
}
