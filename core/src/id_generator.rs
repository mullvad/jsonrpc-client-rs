use jsonrpc_core::types::Id;

#[derive(Debug)]
pub struct IdGenerator {
    next_id: u64,
}

impl IdGenerator {
    pub fn new() -> IdGenerator {
        IdGenerator { next_id: 1 }
    }

    pub fn next(&mut self) -> Id {
        let id = Id::Num(self.next_id);
        self.next_id += 1;
        id
    }

    pub fn next_int(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}
