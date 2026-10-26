use dashmap::DashMap;

#[derive(Default)]
pub struct IdempotencyStore {
    keys: DashMap<String, String>,
}

impl IdempotencyStore {
    pub fn reserve(&self, key: &str, request_id: &str) -> ReserveResult {
        match self.keys.entry(key.to_string()) {
            dashmap::mapref::entry::Entry::Occupied(existing) => {
                ReserveResult::Duplicate(existing.get().clone())
            }
            dashmap::mapref::entry::Entry::Vacant(vacant) => {
                vacant.insert(request_id.to_string());
                ReserveResult::Reserved
            }
        }
    }
}

pub enum ReserveResult {
    Reserved,
    Duplicate(String),
}
