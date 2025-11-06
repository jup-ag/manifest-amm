use crate::quantities::{BaseAtoms, QuoteAtoms};
use bytemuck::{Pod, Zeroable};
use solana_pubkey::Pubkey;
use std::cmp::Ordering;

#[repr(C)]
#[derive(Default, Debug, Copy, Clone, Zeroable, Pod)]
pub struct ClaimedSeat {
    pub trader: Pubkey,
    // Balances are withdrawable on the exchange. They do not include funds in
    // open orders. When moving funds over to open orders, use the worst case
    // rounding.
    pub base_withdrawable_balance: BaseAtoms,
    pub quote_withdrawable_balance: QuoteAtoms,
    /// Quote volume traded over lifetime, can overflow. Double counts self
    /// trades. This is for informational and monitoring purposes only. This is
    /// not guaranteed to be maintained. It does not secure any value in
    /// manifest. Use at your own risk.
    pub quote_volume: QuoteAtoms,
    _padding: [u8; 8],
}

impl ClaimedSeat {
    pub fn new_empty(trader: Pubkey) -> Self {
        ClaimedSeat {
            trader,
            ..Default::default()
        }
    }
}

impl Ord for ClaimedSeat {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.trader).cmp(&(other.trader))
    }
}

impl PartialOrd for ClaimedSeat {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for ClaimedSeat {
    fn eq(&self, other: &Self) -> bool {
        (self.trader) == (other.trader)
    }
}

impl Eq for ClaimedSeat {}

impl std::fmt::Display for ClaimedSeat {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.trader)
    }
}
