use crate::{
    constants::NO_EXPIRATION_LAST_VALID_SLOT,
    quantities::{BaseAtoms, PriceConversionError, QuoteAtomsPerBaseAtom, u64_slice_to_u128},
};
use bytemuck::{Pod, Zeroable};
use hypertree::{DataIndex, PodBool};
use solana_program_error::{ProgramError, ProgramResult};
use std::cmp::Ordering;

#[derive(Debug, PartialEq, Clone, Copy)]
#[repr(u8)]
pub enum OrderType {
    // Normal limit order.
    Limit = 0,

    // Does not rest. Take only.
    ImmediateOrCancel = 1,

    // Fails if would cross the orderbook.
    PostOnly = 2,

    // Global orders are post only but use funds from the global account.
    Global = 3,

    // Reverse orders behave like an AMM. When filled, they place an order on
    // the other side of the book with a small fee (spread).
    // Note: reverse orders can take but don't reverse when taking.
    Reverse = 4,

    // Same as a reverse order except that it is much tighter, allowing for
    // stables to have even smaller spreads.
    ReverseTight = 5,
}
unsafe impl bytemuck::Zeroable for OrderType {}
unsafe impl bytemuck::Pod for OrderType {}
impl Default for OrderType {
    fn default() -> Self {
        OrderType::Limit
    }
}
impl OrderType {
    pub fn is_reversible(self) -> bool {
        matches!(self, OrderType::Reverse | OrderType::ReverseTight)
    }
}

#[repr(C)]
#[derive(Default, Debug, Copy, Clone, Zeroable, Pod)]
pub struct RestingOrder {
    price: QuoteAtomsPerBaseAtom,
    num_base_atoms: BaseAtoms,
    sequence_number: u64,
    trader_index: DataIndex,
    last_valid_slot: u32,
    is_bid: PodBool,
    order_type: OrderType,
    // Spread for reverse orders. Defaults to zero.
    reverse_spread: u16,
    _padding: [u8; 20],
}

impl RestingOrder {
    pub fn new(
        trader_index: DataIndex,
        num_base_atoms: BaseAtoms,
        price: QuoteAtomsPerBaseAtom,
        sequence_number: u64,
        last_valid_slot: u32,
        is_bid: bool,
        order_type: OrderType,
    ) -> Result<Self, ProgramError> {
        // Reverse orders cannot have expiration. The purpose of those orders is to
        // be a permanent liquidity on the book.
        assert!(
            !(order_type.is_reversible() && last_valid_slot != NO_EXPIRATION_LAST_VALID_SLOT)
        );

        Ok(RestingOrder {
            trader_index,
            num_base_atoms,
            last_valid_slot,
            price,
            sequence_number,
            is_bid: PodBool::from_bool(is_bid),
            order_type,
            reverse_spread: 0,
            _padding: Default::default(),
        })
    }

    pub fn get_trader_index(&self) -> DataIndex {
        self.trader_index
    }

    pub fn get_num_base_atoms(&self) -> BaseAtoms {
        self.num_base_atoms
    }

    pub fn get_price(&self) -> QuoteAtomsPerBaseAtom {
        self.price
    }

    pub fn get_order_type(&self) -> OrderType {
        self.order_type
    }

    pub fn is_global(&self) -> bool {
        self.order_type == OrderType::Global
    }

    pub fn is_reverse(&self) -> bool {
        self.order_type.is_reversible()
    }

    pub fn is_reversible(&self) -> bool {
        self.order_type.is_reversible()
    }

    pub fn reverse_price(&self) -> Result<QuoteAtomsPerBaseAtom, PriceConversionError> {
        let base = match self.order_type {
            OrderType::Reverse => 100_000_u32,
            OrderType::ReverseTight => 100_000_000_u32,
            _ => return Ok(self.price),
        };

        if self.get_is_bid() {
            // Bid @P * (1 - spread) --> Ask @P
            // equivalent to
            // Bid @P --> Ask @P / (1 - spread)
            self.price
                .checked_multiply_rational(base, base - self.reverse_spread as u32, false)
        } else {
            // Ask @P --> Bid @P * (1 - spread)
            self.price
                .checked_multiply_rational(base - self.reverse_spread as u32, base, true)
        }
    }

    pub fn get_reverse_spread(self) -> u16 {
        self.reverse_spread
    }

    pub fn set_reverse_spread(&mut self, spread: u16) {
        self.reverse_spread = spread;
    }

    pub fn get_sequence_number(&self) -> u64 {
        self.sequence_number
    }

    pub fn is_expired(&self, current_slot: u32) -> bool {
        self.last_valid_slot != NO_EXPIRATION_LAST_VALID_SLOT && self.last_valid_slot < current_slot
    }

    pub fn get_is_bid(&self) -> bool {
        self.is_bid.0 == 1
    }

    pub fn reduce(&mut self, size: BaseAtoms) -> ProgramResult {
        self.num_base_atoms = self.num_base_atoms.checked_sub(size)?;
        Ok(())
    }

    // Only needed for combining orders. There is no edit_order function.
    pub fn increase(&mut self, size: BaseAtoms) -> ProgramResult {
        self.num_base_atoms = self.num_base_atoms.checked_add(size)?;
        Ok(())
    }
}

impl Ord for RestingOrder {
    fn cmp(&self, other: &Self) -> Ordering {
        // We only compare bids with bids or asks with asks. If you want to
        // check if orders match, directly access their prices.
        debug_assert!(self.get_is_bid() == other.get_is_bid());

        if self.get_is_bid() {
            (self.price).cmp(&other.price)
        } else {
            (other.price).cmp(&(self.price))
        }
    }
}

impl PartialOrd for RestingOrder {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for RestingOrder {
    fn eq(&self, other: &Self) -> bool {
        if self.trader_index != other.trader_index || self.order_type != other.order_type {
            return false;
        }
        if self.order_type.is_reversible() {
            // Allow off by 1 for reverse orders to enable coalescing. Otherwise there is a back and forth that fragments into many orders.
            self.price == other.price
                || u64_slice_to_u128(self.price.inner) + 1 == u64_slice_to_u128(other.price.inner)
                || u64_slice_to_u128(self.price.inner) - 1 == u64_slice_to_u128(other.price.inner)
        } else {
            // Only used in equality check of lookups, so we can ignore size, seqnum, ...
            self.price == other.price
        }
    }
}

impl Eq for RestingOrder {}

impl std::fmt::Display for RestingOrder {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}@{}", self.num_base_atoms, self.price)
    }
}
