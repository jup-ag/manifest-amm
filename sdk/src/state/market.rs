use bytemuck::{Pod, Zeroable};
use hypertree::{
    DataIndex, FreeListNode, Get, HyperTreeValueIteratorTrait, NIL, RBNode, RedBlackTreeReadOnly,
    get_helper,
};
use solana_program_error::{ProgramError, ProgramResult};
use solana_pubkey::Pubkey;

use crate::{
    TypeName, can_back_order,
    constants::MARKET_FIXED_DISCRIMINANT,
    quantities::{BaseAtoms, GlobalAtoms, QuoteAtoms, QuoteAtomsPerBaseAtom, WrapperU64},
    require,
    state::{
        ClaimedSeat, DerefOrBorrow, DynamicAccount,
        resting_order::{OrderType, RestingOrder},
    },
    validation::{GlobalTradeAccounts, ManifestAccount},
};

#[repr(C, packed)]
#[derive(Default, Copy, Clone, Pod, Zeroable)]
pub struct MarketUnusedFreeListPadding {
    _padding: [u64; 9],
    _padding2: [u8; 4],
}

#[repr(C)]
#[derive(Default, Copy, Clone, Zeroable, Pod)]
pub struct MarketFixed {
    /// Discriminant for identifying this type of account.
    pub discriminant: u64,

    /// Version
    version: u8,
    base_mint_decimals: u8,
    quote_mint_decimals: u8,
    base_vault_bump: u8,
    quote_vault_bump: u8,
    _padding1: [u8; 3],

    /// Base mint
    base_mint: Pubkey,
    /// Quote mint
    quote_mint: Pubkey,

    /// Base vault
    base_vault: Pubkey,
    /// Quote vault
    quote_vault: Pubkey,

    /// The sequence number of the next order.
    order_sequence_number: u64,

    /// Num bytes allocated as RestingOrder or ClaimedSeat or FreeList. Does not
    /// include the fixed bytes.
    num_bytes_allocated: u32,

    /// Red-black tree root representing the bids in the order book.
    bids_root_index: DataIndex,
    bids_best_index: DataIndex,

    /// Red-black tree root representing the asks in the order book.
    asks_root_index: DataIndex,
    asks_best_index: DataIndex,

    /// Red-black tree root representing the seats
    claimed_seats_root_index: DataIndex,

    /// LinkedList representing all free blocks that could be used for ClaimedSeats or RestingOrders
    free_list_head_index: DataIndex,

    _padding2: [u32; 1],

    /// Quote volume traded over lifetime, can overflow. This is for
    /// informational and monitoring purposes only. This is not guaranteed to
    /// be maintained. It does not secure any value in manifest.
    /// Use at your own risk.
    quote_volume: QuoteAtoms,

    _padding3: [u64; 8],
}

impl TypeName for MarketFixed {
    const NAME: &'static str = "manifest::state::market::MarketFixed";
}

impl Get for MarketFixed {}

impl ManifestAccount for MarketFixed {
    fn verify_discriminant(&self) -> ProgramResult {
        require!(
            self.discriminant == MARKET_FIXED_DISCRIMINANT,
            ProgramError::InvalidAccountData,
            "Invalid market discriminant actual: {} expected: {}",
            self.discriminant,
            MARKET_FIXED_DISCRIMINANT
        )?;
        Ok(())
    }
}

impl MarketFixed {
    pub fn get_base_mint(&self) -> &Pubkey {
        &self.base_mint
    }
    pub fn get_quote_mint(&self) -> &Pubkey {
        &self.quote_mint
    }
    pub fn get_base_vault(&self) -> &Pubkey {
        &self.base_vault
    }
    pub fn get_quote_vault(&self) -> &Pubkey {
        &self.quote_vault
    }
    pub fn get_base_mint_decimals(&self) -> u8 {
        self.base_mint_decimals
    }
    pub fn get_quote_mint_decimals(&self) -> u8 {
        self.quote_mint_decimals
    }
    pub fn get_base_vault_bump(&self) -> u8 {
        self.base_vault_bump
    }
    pub fn get_quote_vault_bump(&self) -> u8 {
        self.quote_vault_bump
    }
    pub fn get_quote_volume(&self) -> QuoteAtoms {
        self.quote_volume
    }

    // Used only in this file to construct iterator
    pub(crate) fn get_bids_root_index(&self) -> DataIndex {
        self.bids_root_index
    }
    pub(crate) fn get_asks_root_index(&self) -> DataIndex {
        self.asks_root_index
    }
    pub(crate) fn get_bids_best_index(&self) -> DataIndex {
        self.bids_best_index
    }
    pub(crate) fn get_asks_best_index(&self) -> DataIndex {
        self.asks_best_index
    }
}

pub type BooksideReadOnly<'a> = RedBlackTreeReadOnly<'a, RestingOrder>;

/// Fully owned Market, used in clients that can copy.
pub type MarketValue = DynamicAccount<MarketFixed, Vec<u8>>;
/// Full market reference type.
pub type MarketRef<'a> = DynamicAccount<&'a MarketFixed, &'a [u8]>;
/// Full market reference type.
pub type MarketRefMut<'a> = DynamicAccount<&'a mut MarketFixed, &'a mut [u8]>;

// This generic impl covers MarketRef, MarketRefMut and other
// DynamicAccount variants that allow read access.
impl<Fixed: DerefOrBorrow<MarketFixed>, Dynamic: DerefOrBorrow<[u8]>>
    DynamicAccount<Fixed, Dynamic>
{
    fn borrow_market(&self) -> MarketRef {
        MarketRef {
            fixed: self.fixed.deref_or_borrow(),
            dynamic: self.dynamic.deref_or_borrow(),
        }
    }

    pub fn get_base_mint(&self) -> &Pubkey {
        let DynamicAccount { fixed, .. } = self.borrow_market();
        fixed.get_base_mint()
    }

    pub fn get_quote_mint(&self) -> &Pubkey {
        let DynamicAccount { fixed, .. } = self.borrow_market();
        fixed.get_quote_mint()
    }

    pub fn has_free_block(&self) -> bool {
        let DynamicAccount { fixed, .. } = self.borrow_market();
        let free_list_head_index: DataIndex = fixed.free_list_head_index;
        return free_list_head_index != NIL;
    }

    pub fn has_two_free_blocks(&self) -> bool {
        let DynamicAccount { fixed, dynamic } = self.borrow_market();
        let free_list_head_index: DataIndex = fixed.free_list_head_index;
        if free_list_head_index == NIL {
            return false;
        }
        let free_list_head: &FreeListNode<MarketUnusedFreeListPadding> =
            get_helper::<FreeListNode<MarketUnusedFreeListPadding>>(dynamic, free_list_head_index);
        free_list_head.has_next()
    }

    pub fn impact_quote_atoms_with_slot(
        &self,
        is_bid: bool,
        limit_base_atoms: BaseAtoms,
        global_trade_accounts_opts: &[Option<GlobalTradeAccounts>; 2],
        now_slot: u32,
    ) -> Result<QuoteAtoms, ProgramError> {
        let book: BooksideReadOnly = if is_bid {
            self.get_asks()
        } else {
            self.get_bids()
        };
        let required_global_opt: &Option<GlobalTradeAccounts> = if is_bid {
            &global_trade_accounts_opts[0]
        } else {
            &global_trade_accounts_opts[1]
        };

        let mut total_matched_quote_atoms: QuoteAtoms = QuoteAtoms::ZERO;
        let mut remaining_base_atoms: BaseAtoms = limit_base_atoms;
        for (_, resting_order) in book.iter::<RestingOrder>() {
            // Skip expired orders
            if resting_order.is_expired(now_slot) {
                continue;
            }
            let resting_order_type: OrderType = resting_order.get_order_type();
            if resting_order_type == OrderType::Global && required_global_opt.is_none() {
                // Stop walking if we cannot service the first needed global order.
                break;
            }
            let matched_price: QuoteAtomsPerBaseAtom = resting_order.get_price();
            let resting_base_atoms: BaseAtoms = resting_order.get_num_base_atoms();

            // Either fill the entire resting order, or only the
            // remaining_base_atoms, in which case, this is the last iteration
            let matched_base_atoms: BaseAtoms = resting_base_atoms.min(remaining_base_atoms);
            let did_fully_match_resting_order: bool = remaining_base_atoms >= resting_base_atoms;

            // Number of quote atoms matched exactly. Round in taker favor if
            // fully matching.
            let matched_quote_atoms: QuoteAtoms = matched_price.checked_quote_for_base(
                matched_base_atoms,
                is_bid != did_fully_match_resting_order,
            )?;

            // Skip unbacked global orders.
            if self.is_unbacked_global_order(
                &resting_order,
                is_bid,
                required_global_opt,
                matched_base_atoms,
                matched_quote_atoms,
            ) {
                continue;
            }

            total_matched_quote_atoms =
                total_matched_quote_atoms.checked_add(matched_quote_atoms)?;

            if !did_fully_match_resting_order {
                break;
            }

            // prepare for next iteration
            remaining_base_atoms = remaining_base_atoms.checked_sub(matched_base_atoms)?;
        }

        // Note that when there are not enough orders on the market to use up or
        // to receive the desired number of base atoms, this returns just the
        // full amount on the bookside without differentiating that return.

        return Ok(total_matched_quote_atoms);
    }

    pub fn get_bids(&self) -> BooksideReadOnly {
        let DynamicAccount { dynamic, fixed } = self.borrow_market();
        BooksideReadOnly::new(
            dynamic,
            fixed.get_bids_root_index(),
            fixed.get_bids_best_index(),
        )
    }

    pub fn get_asks(&self) -> BooksideReadOnly {
        let DynamicAccount { dynamic, fixed } = self.borrow_market();
        BooksideReadOnly::new(
            dynamic,
            fixed.get_asks_root_index(),
            fixed.get_asks_best_index(),
        )
    }

    fn is_unbacked_global_order(
        &self,
        resting_order: &RestingOrder,
        is_bid: bool,
        global_trade_accounts_opt: &Option<GlobalTradeAccounts>,
        matched_base_atoms: BaseAtoms,
        matched_quote_atoms: QuoteAtoms,
    ) -> bool {
        if resting_order.get_order_type() == OrderType::Global {
            // If global accounts are needed but not present, give no fill.
            if global_trade_accounts_opt.is_none() {
                return true;
            }
            let has_enough_tokens: bool = can_back_order(
                global_trade_accounts_opt,
                self.get_trader_key_by_index(resting_order.get_trader_index()),
                GlobalAtoms::new(if is_bid {
                    matched_base_atoms.as_u64()
                } else {
                    matched_quote_atoms.as_u64()
                }),
            );
            if !has_enough_tokens {
                return true;
            }
        }
        return false;
    }

    pub fn get_trader_key_by_index(&self, index: DataIndex) -> &Pubkey {
        let DynamicAccount { dynamic, .. } = self.borrow_market();

        &get_helper_seat(dynamic, index).get_value().trader
    }

    pub fn impact_base_atoms_with_slot(
        &self,
        is_bid: bool,
        limit_quote_atoms: QuoteAtoms,
        global_trade_accounts_opts: &[Option<GlobalTradeAccounts>; 2],
        now_slot: u32,
    ) -> Result<BaseAtoms, ProgramError> {
        let book: RedBlackTreeReadOnly<'_, RestingOrder> = if is_bid {
            self.get_asks()
        } else {
            self.get_bids()
        };
        let required_global_opt: &Option<GlobalTradeAccounts> = if is_bid {
            &global_trade_accounts_opts[0]
        } else {
            &global_trade_accounts_opts[1]
        };

        let mut total_matched_base_atoms: BaseAtoms = BaseAtoms::ZERO;
        let mut remaining_quote_atoms: QuoteAtoms = limit_quote_atoms;

        for (_, resting_order) in book.iter::<RestingOrder>() {
            // Skip expired orders.
            if resting_order.is_expired(now_slot) {
                continue;
            }
            let resting_order_type: OrderType = resting_order.get_order_type();
            if resting_order_type == OrderType::Global && required_global_opt.is_none() {
                // Stop walking if we cannot service the first needed global order.
                break;
            }

            let matched_price: QuoteAtomsPerBaseAtom = resting_order.get_price();
            // base_atoms_limit is the number of base atoms that you get if you
            // were to trade all of the remaining quote atoms at the current
            // price. Rounding is done in the taker favor because at the limit,
            // it is a full match. So if you are checking against asks with 100
            // quote remaining against price 1.001, then the answer should be
            // 100, because the rounding is in favor of the taker. It takes 100
            // base atoms to exhaust 100 quote atoms at that price.
            let base_atoms_limit: BaseAtoms =
                matched_price.checked_base_for_quote(remaining_quote_atoms, !is_bid)?;
            // Either fill the entire resting order, or only the
            // base_atoms_limit, in which case, this is the last iteration.
            let matched_base_atoms: BaseAtoms =
                resting_order.get_num_base_atoms().min(base_atoms_limit);
            let did_fully_match_resting_order: bool =
                base_atoms_limit >= resting_order.get_num_base_atoms();
            // Number of quote atoms matched exactly. Round in taker favor if
            // fully matching.
            let matched_quote_atoms: QuoteAtoms = matched_price.checked_quote_for_base(
                matched_base_atoms,
                is_bid != did_fully_match_resting_order,
            )?;

            // Skip unbacked global orders.
            if self.is_unbacked_global_order(
                &resting_order,
                is_bid,
                required_global_opt,
                matched_base_atoms,
                matched_quote_atoms,
            ) {
                continue;
            }

            total_matched_base_atoms = total_matched_base_atoms.checked_add(matched_base_atoms)?;

            if !did_fully_match_resting_order {
                break;
            }

            // Prepare for next iteration
            remaining_quote_atoms = remaining_quote_atoms.checked_sub(matched_quote_atoms)?;

            // we can match exactly in base atoms but also deplete all quote atoms at the same time
            if remaining_quote_atoms == QuoteAtoms::ZERO {
                break;
            }
        }

        // Note that when there are not enough orders on the market to use up or
        // to receive the desired number of quote atoms, this returns just the
        // full amount on the bookside without differentiating that return.

        return Ok(total_matched_base_atoms);
    }
}

/// Read a `RBNode<ClaimedSeat>` in an array of data at a given index.
pub fn get_helper_seat(data: &[u8], index: DataIndex) -> &RBNode<ClaimedSeat> {
    get_helper::<RBNode<ClaimedSeat>>(data, index)
}
