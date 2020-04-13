mod storage;
mod arccell;

use std::cell::Cell;
use std::marker::PhantomData;
use std::ops::{ Deref, DerefMut };
use std::sync::Arc;
use std::sync::atomic::{ self, AtomicUsize };
use parking_lot::{ lock_api::RawMutex as _, RawMutex, RwLock, RwLockReadGuard };
use arccell::ArcOption;
pub use storage::{ ThreadLocal, Storage, Insertable, Allocable };


pub struct Map<T, L, M>
where
    T: Clone,
    L: ThreadLocal<ThreadState>,
    M: Storage<Lock<T>>
{
    clock: AtomicUsize,
    local: L,
    storage: M,
    _phantom: PhantomData<T>
}

pub struct MapRef<'a, T, L, M>
where
    T: Clone,
    L: ThreadLocal<ThreadState>,
    M: Storage<Lock<T>>
{
    map: &'a Map<T, L, M>,
    readers: LocalSet<Arc<Lock<T>>>,
    writers: Vec<Arc<Lock<T>>>,
    values: LocalSet<T>,
    remove: Vec<(Arc<Lock<T>>, M::Key)>,
    _phantom: PhantomData<Cell<()>>
}

struct LocalSet<T>(Vec<T>);

pub struct Guard<'a, T> {
    guard: RwLockReadGuard<'a, LockInner<T>>,
    maybe_value: Option<Arc<T>>
}

pub struct MutGuard<'a, T> {
    value: &'a mut T
}

#[derive(Default)]
pub struct ThreadState {
    local_clock: Cell<usize>
}

pub struct Lock<T> {
    inner: RwLock<LockInner<T>>,
    write_lock: RawMutex
}

struct LockInner<T> {
    new_clock: AtomicUsize,
    new_value: ArcOption<T>,
    value: Arc<T>
}

impl<T, L, M> Map<T, L, M>
where
    T: Clone,
    L: ThreadLocal<ThreadState>,
    M: Storage<Lock<T>>
{
    #[inline]
    pub unsafe fn unchecked_new(tls: L, storage: M) -> Map<T, L, M> {
        Map {
            clock: AtomicUsize::new(1),
            local: tls,
            storage,
            _phantom: PhantomData
        }
    }

    pub fn transaction<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut MapRef<'_, T, L, M>) -> R
    {
        let mut mapref = MapRef {
            map: self,
            readers: LocalSet(Vec::new()),
            writers: Vec::new(),
            values: LocalSet(Vec::new()),
            remove: Vec::new(),
            _phantom: PhantomData
        };

        f(&mut mapref)
    }
}

impl<T, L, M> MapRef<'_, T, L, M>
where
    T: Clone,
    L: ThreadLocal<ThreadState>,
    M: Storage<Lock<T>>
{
    pub fn get<'a>(&'a mut self, key: &M::Key) -> Option<Guard<'a, T>> {
        let MapRef { map, readers, .. } = self;

        let global_lock_val = map.storage.get(key)?;
        let global_lock_val = readers.insert(global_lock_val);
        let global_val = global_lock_val.inner.read();

        let global_clock = map.clock.load(atomic::Ordering::SeqCst);
        map.local.with(|state| state.local_clock.set(global_clock));

        // check clock
        let new_clock = global_val.new_clock.load(atomic::Ordering::SeqCst);
        if new_clock > 0 && global_clock >= new_clock {
            if let Some(new_val) = global_val.new_value.get() {
                Some(Guard { guard: global_val, maybe_value: Some(new_val) })
            } else {
                None
            }
        } else {
            Some(Guard { guard: global_val, maybe_value: None })
        }
    }

    pub fn get_mut<'a>(&'a mut self, key: &M::Key) -> Option<MutGuard<'a, T>> {
        let MapRef { map, writers, values, .. } = self;

        let global_lock_val = map.storage.get(key)?;
        global_lock_val.write_lock.lock();

        let new_value = {
            let global_val = global_lock_val.inner.read();

            let global_clock = map.clock.load(atomic::Ordering::SeqCst);
            map.local.with(|state| state.local_clock.set(global_clock));
            global_val.new_clock.store(global_clock, atomic::Ordering::SeqCst);

            T::clone(&global_val.value)
        };

        writers.push(global_lock_val);
        let value = values.insert(new_value);

        Some(MutGuard { value })
    }

    pub fn remove(&mut self, key: M::Key) -> Option<Arc<T>> {
        let MapRef { map, remove, .. } = self;

        let global_lock_val = map.storage.get(&key)?;
        global_lock_val.write_lock.lock();

        let new_value = {
            let global_val = global_lock_val.inner.read();

            let global_clock = map.clock.load(atomic::Ordering::SeqCst);
            map.local.with(|state| state.local_clock.set(global_clock));
            global_val.new_clock.store(global_clock, atomic::Ordering::SeqCst);

            global_val.value.clone()
        };

        remove.push((global_lock_val, key));

        Some(new_value)
    }
}

impl<'a, T, L, M> Drop for MapRef<'a, T, L, M>
where
    T: Clone,
    L: ThreadLocal<ThreadState>,
    M: Storage<Lock<T>>
{
    /// commit transaction
    fn drop(&mut self) {
        let MapRef { map, writers, values, remove, .. } = self;

        let new_clock = map.local.with(|state| {
            let new_clock = state.local_clock.get() + 1;
            state.local_clock.set(new_clock);
            new_clock
        });

        // pre commit
        for (lock, val) in writers.iter().zip(values.0.drain(..)) {
            let global_val = lock.inner.read();
            global_val.new_value.set(Some(Arc::new(val)));
            global_val.new_clock.store(new_clock, atomic::Ordering::SeqCst);
        }
        for (lock, _) in remove.iter() {
            let global_val = lock.inner.read();
            global_val.new_value.set(None);
            global_val.new_clock.store(new_clock, atomic::Ordering::SeqCst);
        }

        // update global clock
        map.clock.fetch_add(1, atomic::Ordering::SeqCst);

        // commit (wait for readers)
        for lock in writers.iter() {
            let mut global_val = lock.inner.write();
            if let Some(val) = global_val.new_value.take() {
                global_val.value = val;
            }
            global_val.new_clock.store(0, atomic::Ordering::SeqCst);
        }
        for (lock, key) in remove.iter() {
            let _global_val = lock.inner.write();
            map.storage.remove(&key);
        }

        // unlock
        for lock in writers.drain(..) {
            lock.write_lock.unlock();
        }
        for (lock, _) in remove.drain(..) {
            lock.write_lock.unlock();
        }
    }
}

impl<T> Deref for Guard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        if let Some(new_value) = &self.maybe_value {
            new_value
        } else {
            &self.guard.value
        }
    }
}

impl<T> Deref for MutGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.value
    }
}

impl<T> DerefMut for MutGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.value
    }
}

impl<T> LocalSet<T> {
    fn insert(&mut self, t: T) -> &mut T {
        self.0.push(t);
        let last = self.0.len() - 1;
        &mut self.0[last]
    }
}

impl<T> Lock<T> {
    pub fn new(t: T) -> Lock<T> {
        Lock {
            inner: RwLock::new(LockInner {
                new_clock: AtomicUsize::new(0),
                new_value: ArcOption::none(),
                value: Arc::new(t)
            }),
            write_lock: RawMutex::INIT
        }
    }
}
