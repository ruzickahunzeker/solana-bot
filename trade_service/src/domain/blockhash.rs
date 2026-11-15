#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Blockhash([u8; 32]);

impl Blockhash {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn to_bytes(self) -> [u8; 32] {
        self.0
    }
}
