use std::sync::Arc;
use std::thread::LocalKey;
use baatzan::{ ThreadLocal, Storage, Map, ThreadState, Lock };


struct GlobalThreadLocal<T: 'static>(&'static LocalKey<T>);
struct StaticArray<T>([Arc<T>; 4]);

impl<T> ThreadLocal<T> for GlobalThreadLocal<T> {
    fn with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&T) -> R
    {
        self.0.with(f)
    }
}

impl<T> Storage<T> for StaticArray<T> {
    type Key = usize;

    fn get(&self, key: &Self::Key) -> Option<Arc<T>> {
        self.0.get(*key).cloned()
    }

    fn remove(&self, _key: &Self::Key) -> Option<Arc<T>> {
        None
    }
}

#[test]
fn test_simple() {
    thread_local!(
        static LOCAL: ThreadState = ThreadState::default()
    );

    let map = unsafe {
        Map::unchecked_new(
            GlobalThreadLocal(&LOCAL),
            StaticArray([
                Arc::new(Lock::new(0)),
                Arc::new(Lock::new(1)),
                Arc::new(Lock::new(2)),
                Arc::new(Lock::new(3)),
            ])
        )
    };

    let map = Arc::new(map);


    map.transaction(|map| {
        let val = map.get(&0).unwrap();
        assert_eq!(*val, 0);
        drop(val);

        let mut val = map.get_mut(&1).unwrap();
        *val = 4;
    });

    map.transaction(|map| {
        let val = map.get(&1).unwrap();
        assert_eq!(*val, 4);
    });

    //
}
