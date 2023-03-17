//! a set that has been "optomized" for performance with small keys
//TODO: optimization

#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct SmallSet<T: Ord> {
    values: Vec<T>
}

impl<T: Ord> SmallSet<T> {
    pub fn new() -> Self {
        Self {
            values: Vec::default()
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            values: Vec::with_capacity(capacity)
        }
    }

    pub fn insert(&mut self, item: T) -> Option<T> {
        if let Err(idx) = self.values.binary_search(&item) {
            self.values.insert(idx, item);
            None
        } else {
            Some(item)    
        }
    }

    pub fn remove(&mut self, item: &T) -> Option<T> {
        if let Ok(idx) = self.values.binary_search(item) {
            Some(self.values.remove(idx))
        } else {
            None
        }
    }

    pub fn contains(&mut self, item: &T) -> bool {
        self.values.binary_search(item).is_ok()
    }

    pub fn drain(self) -> impl Iterator<Item = T> {
        self.values.into_iter()
    }

    pub fn as_slice(&self) -> &[T] {
        self.values.as_slice()
    }
}

