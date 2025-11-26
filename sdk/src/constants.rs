use hypertree::RBTREE_OVERHEAD_BYTES;

// Account sizes
pub const MARKET_FIXED_SIZE: usize = 256;
pub const GLOBAL_FIXED_SIZE: usize = 96;

// Block sizing for hypertree payloads
pub const GLOBAL_BLOCK_SIZE: usize = 64;
pub const MARKET_BLOCK_SIZE: usize = 80;
const MARKET_BLOCK_PAYLOAD_SIZE: usize = MARKET_BLOCK_SIZE - RBTREE_OVERHEAD_BYTES;
const GLOBAL_BLOCK_PAYLOAD_SIZE: usize = GLOBAL_BLOCK_SIZE - RBTREE_OVERHEAD_BYTES;

pub const RESTING_ORDER_SIZE: usize = MARKET_BLOCK_PAYLOAD_SIZE;
pub const CLAIMED_SEAT_SIZE: usize = MARKET_BLOCK_PAYLOAD_SIZE;
pub const GLOBAL_TRADER_SIZE: usize = GLOBAL_BLOCK_PAYLOAD_SIZE;
pub const GLOBAL_DEPOSIT_SIZE: usize = GLOBAL_BLOCK_PAYLOAD_SIZE;

const FREE_LIST_OVERHEAD: usize = 4;
pub const MARKET_FREE_LIST_BLOCK_SIZE: usize = MARKET_BLOCK_SIZE - FREE_LIST_OVERHEAD;
pub const GLOBAL_FREE_LIST_BLOCK_SIZE: usize = GLOBAL_BLOCK_SIZE - FREE_LIST_OVERHEAD;

pub const NO_EXPIRATION_LAST_VALID_SLOT: u32 = 0;

// Discriminants
pub const MARKET_FIXED_DISCRIMINANT: u64 = 4859840929024028656;
pub const GLOBAL_FIXED_DISCRIMINANT: u64 = 10787423733276977665;

// Gas prepayment for global orders (economic spam deterrent)
pub const GAS_DEPOSIT_LAMPORTS: u64 = 5_000;

/// Limit on the number of global seats available.
#[cfg(feature = "test")]
pub const MAX_GLOBAL_SEATS: u16 = 4;
#[cfg(not(feature = "test"))]
pub const MAX_GLOBAL_SEATS: u16 = 999;
