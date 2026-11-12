use bytemuck::{Pod, Zeroable};
use hypertree::{
    DataIndex, FreeListNode, Get, HyperTreeValueIteratorTrait, NIL, RBNode, RedBlackTreeReadOnly,
    get_helper,
};
use solana_program_error::{ProgramError, ProgramResult};
use solana_pubkey::Pubkey;
use std::collections::HashMap;

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

unsafe impl Get for MarketFixed {}

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
    fn borrow_market(&self) -> MarketRef<'_> {
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
        free_list_head_index != NIL
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

        // Global atoms promised per maker so far in this walk.
        let mut global_used: HashMap<Pubkey, GlobalAtoms> = HashMap::new();
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
                resting_order,
                is_bid,
                required_global_opt,
                matched_base_atoms,
                matched_quote_atoms,
                &mut global_used,
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

        Ok(total_matched_quote_atoms)
    }

    pub fn get_bids(&self) -> BooksideReadOnly<'_> {
        let DynamicAccount { dynamic, fixed } = self.borrow_market();
        BooksideReadOnly::new(
            dynamic,
            fixed.get_bids_root_index(),
            fixed.get_bids_best_index(),
        )
    }

    pub fn get_asks(&self) -> BooksideReadOnly<'_> {
        let DynamicAccount { dynamic, fixed } = self.borrow_market();
        BooksideReadOnly::new(
            dynamic,
            fixed.get_asks_root_index(),
            fixed.get_asks_best_index(),
        )
    }

    /// `global_used` is this walk's ledger of global atoms already promised
    /// to earlier fills, per maker. On chain every global fill moves tokens
    /// out of the maker's global deposit, so a later order from the same maker
    /// only sees what is left; an order the remainder cannot cover is removed
    /// without moving anything. Checking each order against the full deposit
    /// instead would count the same balance once per order.
    fn is_unbacked_global_order(
        &self,
        resting_order: &RestingOrder,
        is_bid: bool,
        global_trade_accounts_opt: &Option<GlobalTradeAccounts>,
        matched_base_atoms: BaseAtoms,
        matched_quote_atoms: QuoteAtoms,
        global_used: &mut HashMap<Pubkey, GlobalAtoms>,
    ) -> bool {
        if resting_order.get_order_type() == OrderType::Global {
            // If global accounts are needed but not present, give no fill.
            if global_trade_accounts_opt.is_none() {
                return true;
            }
            let maker: &Pubkey = self.get_trader_key_by_index(resting_order.get_trader_index());
            // When is_bid, the taker supplies quote, so the global maker
            // supplies base.
            let desired_global_atoms: GlobalAtoms = GlobalAtoms::new(if is_bid {
                matched_base_atoms.as_u64()
            } else {
                matched_quote_atoms.as_u64()
            });
            let already_used: GlobalAtoms = global_used.get(maker).copied().unwrap_or_default();
            let has_enough_tokens: bool = can_back_order(
                global_trade_accounts_opt,
                maker,
                already_used,
                desired_global_atoms,
            );
            if !has_enough_tokens {
                return true;
            }
            // can_back_order already proved already_used + desired fits in
            // the deposit, so this cannot overflow.
            global_used.insert(*maker, already_used.wrapping_add(desired_global_atoms));
        }
        false
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

        // Global atoms promised per maker so far in this walk.
        let mut global_used: HashMap<Pubkey, GlobalAtoms> = HashMap::new();
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
            //
            // A limit beyond u64::MAX base atoms (a dust-priced resting order
            // against a large remaining quote) is capped rather than
            // propagated: the min() below bounds the fill by the resting
            // order's actual size either way, and failing instead would let
            // one dust-priced resting order error every quote-denominated
            // swap on the market. (upstream #751)
            let base_atoms_limit: BaseAtoms = matched_price
                .checked_base_for_quote(remaining_quote_atoms, !is_bid)
                .unwrap_or(BaseAtoms::new(u64::MAX));
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
                resting_order,
                is_bid,
                required_global_opt,
                matched_base_atoms,
                matched_quote_atoms,
                &mut global_used,
            ) {
                continue;
            }

            total_matched_base_atoms = total_matched_base_atoms.checked_add(matched_base_atoms)?;

            // A complete fill can round its quote proceeds above the remaining
            // target (one base atom at price 1.5 charges two quote atoms
            // against a remaining target of one). The target is reached, so
            // stop before the subtraction below underflows. (upstream #751)
            if !did_fully_match_resting_order || matched_quote_atoms >= remaining_quote_atoms {
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

        Ok(total_matched_base_atoms)
    }
}

/// Read a `RBNode<ClaimedSeat>` in an array of data at a given index.
pub fn get_helper_seat(data: &[u8], index: DataIndex) -> &RBNode<ClaimedSeat> {
    get_helper::<RBNode<ClaimedSeat>>(data, index)
}

#[cfg(test)]
mod global_backing_tests {
    use super::*;
    use crate::{
        constants::{GLOBAL_BLOCK_SIZE, MARKET_BLOCK_SIZE, NO_EXPIRATION_LAST_VALID_SLOT},
        state::{GlobalDeposit, GlobalFixed, GlobalTrader},
        validation::ManifestAccountInfo,
    };
    use bytemuck::Zeroable;
    use hypertree::{
        HyperTreeReadOperations, HyperTreeWriteOperations, RedBlackTree, get_mut_helper,
    };
    use solana_account_info::AccountInfo;

    struct Order {
        maker: usize,
        /// Price as mantissa * 10^exponent quote atoms per base atom.
        price: (u32, i8),
        base_atoms: u64,
        order_type: OrderType,
    }

    fn global(maker: usize, price: (u32, i8), base_atoms: u64) -> Order {
        Order {
            maker,
            price,
            base_atoms,
            order_type: OrderType::Global,
        }
    }

    fn limit(maker: usize, price: (u32, i8), base_atoms: u64) -> Order {
        Order {
            maker,
            price,
            base_atoms,
            order_type: OrderType::Limit,
        }
    }

    /// A market whose seats are `makers` and whose `is_bid` side holds `orders`.
    fn build_market(makers: &[Pubkey], is_bid: bool, orders: &[Order]) -> MarketValue {
        let mut dynamic: Vec<u8> = vec![0; (makers.len() + orders.len()) * MARKET_BLOCK_SIZE];
        let block = |i: usize| (i * MARKET_BLOCK_SIZE) as DataIndex;

        let mut seats: RedBlackTree<ClaimedSeat> = RedBlackTree::new(&mut dynamic, NIL, NIL);
        for (i, maker) in makers.iter().enumerate() {
            seats.insert(block(i), ClaimedSeat::new_empty(*maker));
        }
        let seats_root: DataIndex = seats.get_root_index();

        let mut book: RedBlackTree<RestingOrder> = RedBlackTree::new(&mut dynamic, NIL, NIL);
        for (i, order) in orders.iter().enumerate() {
            let price =
                QuoteAtomsPerBaseAtom::try_from_mantissa_and_exponent(order.price.0, order.price.1)
                    .unwrap();
            let resting_order = RestingOrder::new(
                block(order.maker),
                BaseAtoms::new(order.base_atoms),
                price,
                i as u64,
                NO_EXPIRATION_LAST_VALID_SLOT,
                is_bid,
                order.order_type,
            )
            .unwrap();
            book.insert(block(makers.len() + i), resting_order);
        }
        let (book_root, book_best) = (book.get_root_index(), book.get_max_index());

        let mut fixed: MarketFixed = MarketFixed::zeroed();
        fixed.discriminant = MARKET_FIXED_DISCRIMINANT;
        fixed.claimed_seats_root_index = seats_root;
        fixed.free_list_head_index = NIL;
        (
            fixed.bids_root_index,
            fixed.bids_best_index,
            fixed.asks_root_index,
            fixed.asks_best_index,
        ) = if is_bid {
            (book_root, book_best, NIL, NIL)
        } else {
            (NIL, NIL, book_root, book_best)
        };
        MarketValue { fixed, dynamic }
    }

    /// Serialized global account where `makers[i]` has `balances[i]` deposited.
    fn build_global_bytes(mint: &Pubkey, makers: &[Pubkey], balances: &[u64]) -> Vec<u8> {
        let mut dynamic: Vec<u8> = vec![0; 2 * makers.len() * GLOBAL_BLOCK_SIZE];
        let block = |i: usize| (i * GLOBAL_BLOCK_SIZE) as DataIndex;

        let mut deposits: RedBlackTree<GlobalDeposit> = RedBlackTree::new(&mut dynamic, NIL, NIL);
        for (i, (maker, balance)) in makers.iter().zip(balances).enumerate() {
            deposits.insert(
                block(makers.len() + i),
                GlobalDeposit::new_for_test(maker, GlobalAtoms::new(*balance)),
            );
        }
        let mut traders: RedBlackTree<GlobalTrader> = RedBlackTree::new(&mut dynamic, NIL, NIL);
        for (i, maker) in makers.iter().enumerate() {
            traders.insert(
                block(i),
                GlobalTrader::new_empty(maker, block(makers.len() + i)),
            );
        }
        let traders_root: DataIndex = traders.get_root_index();

        let mut bytes: Vec<u8> = vec![0; size_of::<GlobalFixed>()];
        let fixed: &mut GlobalFixed = get_mut_helper::<GlobalFixed>(&mut bytes, 0);
        *fixed = GlobalFixed::new_empty(mint);
        fixed.set_global_traders_root_index_for_test(traders_root);
        bytes.extend_from_slice(&dynamic);
        bytes
    }

    /// Run `f` with global trade accounts in the slot the walk reads for
    /// `is_bid` (base global for bids, quote global for asks).
    fn with_global<R>(
        is_bid: bool,
        global_bytes: &mut [u8],
        f: impl FnOnce(&[Option<GlobalTradeAccounts>; 2]) -> R,
    ) -> R {
        let key = Pubkey::new_unique();
        let mut lamports: u64 = 0;
        let info = AccountInfo::new(
            &key,
            false,
            false,
            &mut lamports,
            global_bytes,
            &crate::ID,
            false,
        );
        let accounts = GlobalTradeAccounts {
            mint_opt: None,
            global: ManifestAccountInfo::new(&info).unwrap(),
            global_vault_opt: None,
            market_vault_opt: None,
            token_program_opt: None,
            system_program: None,
            gas_payer_opt: None,
            gas_receiver_opt: None,
            market: Pubkey::new_unique(),
            global_transfer_blocked: false,
        };
        let opts: [Option<GlobalTradeAccounts>; 2] = if is_bid {
            [Some(accounts), None]
        } else {
            [None, Some(accounts)]
        };
        f(&opts)
    }

    /// Quote atoms received for selling `base_atoms` into a bid book.
    fn sell_base(makers: &[Pubkey], balances: &[u64], bids: &[Order], base_atoms: u64) -> u64 {
        let market = build_market(makers, true, bids);
        let mut global = build_global_bytes(&Pubkey::new_unique(), makers, balances);
        with_global(false, &mut global, |opts| {
            market
                .impact_quote_atoms_with_slot(false, BaseAtoms::new(base_atoms), opts, 0)
                .unwrap()
                .as_u64()
        })
    }

    #[test]
    fn same_maker_global_orders_share_one_deposit() {
        let makers = [Pubkey::new_unique()];
        // 100 base @ 2 needs 200 quote, then 100 base @ 1 needs 100 more. A
        // 250 deposit covers the first only: on chain the second is removed
        // unfilled. Checked independently, each fits in 250 and the quote
        // would report 300.
        let bids = [global(0, (2, 0), 100), global(0, (1, 0), 100)];
        assert_eq!(sell_base(&makers, &[250], &bids, 200), 200);
    }

    #[test]
    fn different_makers_are_backed_independently() {
        let makers = [Pubkey::new_unique(), Pubkey::new_unique()];
        let bids = [global(0, (2, 0), 100), global(1, (1, 0), 100)];
        assert_eq!(sell_base(&makers, &[250, 250], &bids, 200), 300);
    }

    #[test]
    fn non_global_orders_are_not_charged_to_the_ledger() {
        let makers = [Pubkey::new_unique(), Pubkey::new_unique()];
        // Maker 0 global 100 @ 2 (200), maker 1 limit 100 @ 1.5 (150), maker 0
        // global 100 @ 1 (would need 300 total from a 250 deposit: skipped).
        let bids = [
            global(0, (2, 0), 100),
            limit(1, (15, -1), 100),
            global(0, (1, 0), 100),
        ];
        assert_eq!(sell_base(&makers, &[250, 0], &bids, 300), 350);
    }

    #[test]
    fn exhausting_one_maker_does_not_affect_another() {
        let makers = [Pubkey::new_unique(), Pubkey::new_unique()];
        // Maker 0 has 200: the 200 fill uses it all, so its 150 order is
        // skipped; maker 1's 100 order is still backed by its own deposit.
        let bids = [
            global(0, (2, 0), 100),
            global(0, (15, -1), 100),
            global(1, (1, 0), 100),
        ];
        assert_eq!(sell_base(&makers, &[200, 100], &bids, 300), 300);
    }

    #[test]
    fn partial_last_fill_only_needs_what_it_takes() {
        let makers = [Pubkey::new_unique()];
        // 250 deposit: the first order takes 200, the second only fills 50
        // base @ 1 = 50 quote, which still fits.
        let bids = [global(0, (2, 0), 100), global(0, (1, 0), 100)];
        assert_eq!(sell_base(&makers, &[250], &bids, 150), 250);
    }

    #[test]
    fn buying_base_charges_the_base_deposit() {
        let makers = [Pubkey::new_unique()];
        // Asks: maker sells 100 base @ 1 then 100 base @ 2 from a 150 base
        // deposit, so only the first is backed.
        let asks = [global(0, (1, 0), 100), global(0, (2, 0), 100)];
        let market = build_market(&makers, false, &asks);
        let mut global_bytes = build_global_bytes(&Pubkey::new_unique(), &makers, &[150]);
        let base_out = with_global(true, &mut global_bytes, |opts| {
            market
                .impact_base_atoms_with_slot(true, QuoteAtoms::new(1_000), opts, 0)
                .unwrap()
                .as_u64()
        });
        assert_eq!(base_out, 100);
    }
}
