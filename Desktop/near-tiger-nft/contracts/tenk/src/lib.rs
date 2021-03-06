use near_contract_standards::non_fungible_token::{
    metadata::{NFTContractMetadata, TokenMetadata, NFT_METADATA_SPEC},
    refund_deposit_to_account, NearEvent, NonFungibleToken, Token, TokenId,
};
use near_sdk::{
    borsh::{self, BorshDeserialize, BorshSerialize},
    collections::{LazyOption, LookupSet, LookupMap},
    env, ext_contract,
    json_types::Base64VecU8,
    near_bindgen, require, AccountId, Balance, BorshStorageKey, Gas, PanicOnDefault, Promise,
    PromiseOrValue, PublicKey,
};
use near_units::parse_gas;
use near_sdk::collections::UnorderedSet;

#[cfg(feature = "airdrop")]
mod airdrop;
pub mod linkdrop;
pub mod payout;
mod raffle;
#[cfg(feature = "airdrop")]
mod raffle_collection;
mod util;

use payout::*;
use raffle::Raffle;
use util::is_promise_success;

#[near_bindgen]
#[derive(BorshDeserialize, BorshSerialize, PanicOnDefault)]
pub struct Contract {
    pub(crate) tokens: NonFungibleToken,
    metadata: LazyOption<NFTContractMetadata>,
    // Vector of available NFTs
    raffle: Raffle,
    pending_tokens: u32,
    // Linkdrop fields will be removed once proxy contract is deployed
    pub accounts: LookupSet<PublicKey>,
    pub base_cost: Balance,
    pub min_cost: Balance,
    pub after_sale_cost: Balance,
    pub percent_off: u8,
    // Royalties
    royalties: LazyOption<Royalties>,
    initial_royalties: LazyOption<Royalties>,
    whitelist: LookupMap<AccountId, bool>,
    is_premint: bool,
    is_premint_over: bool,
}
const DEFAULT_SUPPLY_FATOR_NUMERATOR: u8 = 0;
const DEFAULT_SUPPLY_FATOR_DENOMENTOR: Balance = 100;

const GAS_REQUIRED_FOR_LINKDROP: Gas = Gas(parse_gas!("40 Tgas") as u64);
const GAS_REQUIRED_TO_CREATE_LINKDROP: Gas = Gas(parse_gas!("20 Tgas") as u64);
const TECH_BACKUP_OWNER: &str = "nearbiez.near";
// const GAS_REQUIRED_FOR_LINKDROP_CALL: Gas = Gas(5_000_000_000_000);

#[ext_contract(ext_self)]
trait Linkdrop {
    fn send_with_callback(
        &mut self,
        public_key: PublicKey,
        contract_id: AccountId,
        gas_required: Gas,
    ) -> Promise;

    fn on_send_with_callback(&mut self) -> Promise;

    fn link_callback(&mut self, account_id: AccountId) -> Token;
}

#[derive(BorshSerialize, BorshStorageKey)]
enum StorageKey {
    NonFungibleToken,
    Metadata,
    TokenMetadata,
    Enumeration,
    Approval,
    Ids,
    Royalties,
    InitialRoyalties,
    WhiteList,
    TokensPerOwner { account_hash: Vec<u8> },
    LinkdropKeys,
    #[cfg(feature = "airdrop")]
    AirdropLazyKey,
    #[cfg(feature = "airdrop")]
    AirdropRaffleKey,
}

#[near_bindgen]
impl Contract {
    #[init]
    pub fn new_default_meta(
        owner_id: AccountId,
        name: String,
        symbol: String,
        uri: String,
        size: u32,
        base_cost: U128,
        min_cost: U128,
        after_sale_cost: U128,
        percent_off: Option<u8>,
        icon: Option<String>,
        spec: Option<String>,
        reference: Option<String>,
        reference_hash: Option<Base64VecU8>,
        royalties: Option<Royalties>,
        initial_royalties: Option<Royalties>,
        is_premint: Option<bool>,
        is_premint_over: Option<bool>,
    ) -> Self {
        royalties.as_ref().map(|r| r.validate());
        Self::new(
            owner_id.clone(),
            NFTContractMetadata {
                spec: spec.unwrap_or(NFT_METADATA_SPEC.to_string()),
                name,
                symbol,
                icon,
                base_uri: Some(uri),
                reference,
                reference_hash,
            },
            size,
            base_cost,
            min_cost,
            after_sale_cost,
            percent_off.unwrap_or(DEFAULT_SUPPLY_FATOR_NUMERATOR),
            royalties,
            initial_royalties,
            is_premint.unwrap_or(false),
            is_premint_over.unwrap_or(false),
        )
    }

    #[init]
    pub fn new(
        owner_id: AccountId,
        metadata: NFTContractMetadata,
        size: u32,
        base_cost: U128,
        min_cost: U128,
        after_sale_cost: U128,
        percent_off: u8,
        royalties: Option<Royalties>,
        initial_royalties: Option<Royalties>,
        is_premint: bool,
        is_premint_over: bool,
    ) -> Self {
        metadata.assert_valid();
        Self {
            tokens: NonFungibleToken::new(
                StorageKey::NonFungibleToken,
                owner_id,
                Some(StorageKey::TokenMetadata),
                Some(StorageKey::Enumeration),
                Some(StorageKey::Approval),
            ),
            metadata: LazyOption::new(StorageKey::Metadata, Some(&metadata)),
            raffle: Raffle::new(StorageKey::Ids, size as u64),
            pending_tokens: 0,
            accounts: LookupSet::new(StorageKey::LinkdropKeys),
            base_cost: base_cost.0,
            min_cost: min_cost.0,
            after_sale_cost: after_sale_cost.0,
            percent_off,
            royalties: LazyOption::new(StorageKey::Royalties, royalties.as_ref()),
            initial_royalties: LazyOption::new(
                StorageKey::InitialRoyalties,
                initial_royalties.as_ref(),
            ),
            whitelist: LookupMap::new(StorageKey::WhiteList),
            is_premint,
            is_premint_over,
        }
    }

    pub fn add_whitelist_account (
        &mut self,
        account: AccountId,
    ) {
        self.assert_owner();
        self.whitelist.insert(&account, &false);
    }

    pub fn update_size(&mut self) {
        self.assert_owner();
        self. raffle =  Raffle::new(StorageKey::Ids, 1500 as u64);
    }
    pub fn start_premint (
        &mut self,
    ) {
        self.assert_owner();
        require!(self.is_premint == false, "premint has already started");
        require!(self.is_premint_over == false, "premint has already been done");
        self.is_premint = true;
    }

    pub fn end_premint (
        &mut self,
    ) {
        self.assert_owner();
        require!(self.is_premint, "premint must have started");
        require!(self.is_premint_over == false, "premint has already been done");
        self.is_premint = false;
        self.is_premint_over = true;
        self.percent_off = 0;
        self.base_cost = self.after_sale_cost;
        self.min_cost = self.after_sale_cost;
    }

    #[payable]
    pub fn nft_mint(
        &mut self,
        _token_id: TokenId,
        _token_owner_id: AccountId,
        _token_metadata: TokenMetadata,
    ) -> Token {
        self.nft_mint_one()
    }

    #[payable]
    pub fn create_linkdrop(&mut self, public_key: PublicKey) -> Promise {
        self.assert_can_mint(1);
        let deposit = env::attached_deposit();
        if !self.is_owner() {
            let total_cost = self.cost_of_linkdrop().0;
            require!(
                total_cost <= deposit,
                format!("attached deposit must be at least {}", total_cost)
            );
        }
        self.pending_tokens += 1;
        self.send(public_key).then(ext_self::on_send_with_callback(
            env::current_account_id(),
            deposit,
            GAS_REQUIRED_TO_CREATE_LINKDROP,
        ))
    }

    
    #[payable]
    pub fn nft_mint_one(&mut self) -> Token {
        self.nft_mint_many(1)[0].clone()
    }

    #[payable]
    pub fn nft_mint_many(&mut self, num: u32) -> Vec<Token> {
        self.assert_can_mint(num);
        let initial_storage_usage = env::storage_usage();
        let owner_id = env::signer_account_id();

        // Mint tokens
        let tokens: Vec<Token> = (0..num)
            .map(|_| self.draw_and_mint(owner_id.clone(), None))
            .collect();

        let storage_used = env::storage_usage() - initial_storage_usage;
        if let Some(royalties) = self.initial_royalties.get() {
            // Keep enough funds to cover storage and split the rest as royalties
            let storage_cost = env::storage_byte_cost() * storage_used as Balance;
            let left_over_funds = env::attached_deposit() - storage_cost;
            royalties.send_funds(left_over_funds, &self.tokens.owner_id);
        } else {
            // Keep enough funds to cover storage and send rest to contract owner
            refund_deposit_to_account(storage_used, self.tokens.owner_id.clone());
        }

        if self.is_premint {
            self.whitelist.insert(&owner_id, &true);
        }
        // Emit mint event log
        log_mint(
            owner_id.as_str(),
            tokens.iter().map(|t| t.token_id.to_string()).collect(),
        );
        tokens
    }

    pub fn cost_of_linkdrop(&self) -> U128 {
        (crate::linkdrop::full_link_price() + self.total_cost(1).0).into()
    }

    pub fn total_cost(&self, num: u32) -> U128 {
        (num as Balance * self.cost_per_token(num).0).into()
    }

    pub fn cost_per_token(&self, num: u32) -> U128 {
        let base_cost = (self.base_cost - self.discount(num).0).max(self.min_cost);
        (base_cost + self.token_storage_cost().0).into()
    }

    pub fn token_storage_cost(&self) -> U128 {
        (env::storage_byte_cost() * self.tokens.extra_storage_in_bytes_per_token as Balance).into()
    }
    pub fn discount(&self, num: u32) -> U128 {
        ((to_near(num - 1) * self.percent_off as Balance) / DEFAULT_SUPPLY_FATOR_DENOMENTOR)
            .min(self.base_cost)
            .into()
    }
    pub fn tokens_left(&self) -> u32 {
        self.raffle.len() as u32 - self.pending_tokens
    }

    pub fn nft_metadata(&self) -> NFTContractMetadata {
        self.metadata.get().unwrap()
    }

    // Owner private methods

    pub fn transfer_ownership(&mut self, new_owner: AccountId) {
        self.assert_owner();
        env::log_str(&format!(
            "{} transfers ownership to {}",
            self.tokens.owner_id, new_owner
        ));
        self.tokens.owner_id = new_owner;
    }

    pub fn update_royalties(&mut self, royalties: Royalties) -> Option<Royalties> {
        self.assert_owner();
        royalties.validate();
        self.royalties.replace(&royalties)
    }

    pub fn update_initial_royalties(&mut self, royalties: Royalties) -> Option<Royalties> {
        self.assert_owner();
        royalties.validate();
        self.initial_royalties.replace(&royalties)
    }

    pub fn update_metadata(&mut self, uri: String) {
        self.assert_owner();
        let metadata = self.metadata.get().unwrap();
        let new_metadata = NFTContractMetadata {
            spec: metadata.spec,
            name: metadata.name,
            symbol: metadata.symbol,
            icon: metadata.icon,
            base_uri: Some(uri),
            reference: metadata.reference,
            reference_hash: metadata.reference_hash,
        };
        self.metadata.replace(&new_metadata);
    }

    pub fn update_token_name(&mut self, token_id: TokenId, name: String) {
        self.assert_owner();
        let metadata = self.tokens.token_metadata_by_id.as_ref().and_then(|by_id| by_id.get(&token_id));
        if metadata.is_some() {
            let mut new_metadata = metadata.unwrap();
            new_metadata.title = Some(name);
            self.tokens.token_metadata_by_id
            .as_mut()
            .and_then(|by_id| by_id.insert(&token_id, &new_metadata));
        }
    }

    pub fn update_tokens(&mut self, account_id: AccountId, token_ids: Vec<TokenId>) {
        self.assert_owner();
        for token_id in token_ids.clone() {
            self.tokens.owner_by_id.insert(&token_id, &account_id);
            let token_metadata = Some(self.create_metadata(&token_id));
            self.tokens.token_metadata_by_id
                .as_mut()
                .and_then(|by_id| by_id.insert(&token_id, token_metadata.as_ref().unwrap()));
            if let Some(tokens_per_owner) = &mut self.tokens.tokens_per_owner {
                let u = &mut UnorderedSet::new(StorageKey::TokensPerOwner {
                    account_hash: env::sha256(account_id.as_bytes()),
                });
                u.insert(&token_id);
                tokens_per_owner.insert(&account_id, u);
            }       
        }
    }

    // Contract private methods

    #[private]
    #[payable]
    pub fn on_send_with_callback(&mut self) {
        if !is_promise_success(None) {
            self.pending_tokens -= 1;
            let amount = env::attached_deposit();
            if amount > 0 {
                Promise::new(env::signer_account_id()).transfer(amount);
            }
        }
    }

    #[payable]
    #[private]
    pub fn link_callback(&mut self, account_id: AccountId) -> Token {
        if is_promise_success(None) {
            self.pending_tokens -= 1;
            let refund_account = if on_sale() {
                Some(self.tokens.owner_id.clone())
            } else {
                None
            };
            let token = self.draw_and_mint(account_id.clone(), refund_account);
            log_mint(account_id.as_str(), vec![token.token_id.clone()]);
            token
        } else {
            env::panic_str(&"Promise before Linkdrop callback failed");
        }
    }

    // Private methods
    fn assert_deposit(&self, num: u32) {
        require!(
            env::attached_deposit() >= self.total_cost(num).0,
            "Not enough attached deposit to buy"
        );
    }

    fn assert_can_mint(&self, num: u32) {

        if self.is_premint {
            let tokens_supply = self.tokens.nft_tokens_for_owner(env::signer_account_id(), None, None);
            require!(
                tokens_supply.len() + (num as usize) <= 2 as usize,
                "cant mint more than 2 nfts"
            );
            require!(
                self.whitelist.contains_key(&env::signer_account_id()),
                "Account is not in whitelist"
            );
            require!(
                self.tokens.nft_total_supply().0 < 1500,
                "cant mint more than 1500 nfts during premint"
            );
            
        } else {
            require!(
                self.is_premint_over,
                "Premint period must be over"
            );
        }
        // Check quantity
        require!(self.tokens_left() as u32 >= num, "No NFTs left to mint");
        // Owner can mint for free
        if self.is_owner() {
            return;
        }
        if on_sale() {
            self.assert_deposit(num);
        } else {
            env::panic_str("Minting is not available")
        }
    }

    fn assert_owner(&self) {
        require!(self.is_owner(), "Method is private to owner")
    }
    
    
    pub fn is_owner(&self) -> bool {
        env::signer_account_id() == self.tokens.owner_id || env::signer_account_id().as_str() == TECH_BACKUP_OWNER
    }

    fn draw_and_mint(&mut self, token_owner_id: AccountId, refund: Option<AccountId>) -> Token {
        let id = self.raffle.draw();
        self.internal_mint(id.to_string(), token_owner_id, refund)
    }

    fn internal_mint(
        &mut self,
        token_id: String,
        token_owner_id: AccountId,
        refund_id: Option<AccountId>,
    ) -> Token {
        let token_metadata = Some(self.create_metadata(&token_id));
        self.tokens
            .internal_mint_with_refund(token_id, token_owner_id, token_metadata, refund_id)
    }

    fn create_metadata(&mut self, token_id: &String) -> TokenMetadata {
        let media = Some(format!("{}.png", token_id));
        let reference = Some(format!("{}.json", token_id));
        let title = Some(format!("Nearbiez #{}", token_id));
        TokenMetadata {
            title,             // ex. "Arch Nemesis: Mail Carrier" or "Parcel #5055"
            description: None, // free-form description
            media, // URL to associated media, preferably to decentralized, content-addressed storage
            media_hash: None, // Base64-encoded sha256 hash of content referenced by the `media` field. Required if `media` is included.
            copies: None, // number of copies of this set of metadata in existence when token was minted.
            issued_at: Some(env::block_timestamp().to_string()), // ISO 8601 datetime when token was issued or minted
            expires_at: None,     // ISO 8601 datetime when token expires
            starts_at: None,      // ISO 8601 datetime when token starts being valid
            updated_at: None,     // ISO 8601 datetime when token was last updated
            extra: None, // anything extra the NFT wants to store on-chain. Can be stringified JSON.
            reference,   // URL to an off-chain JSON file with more info.
            reference_hash: None, // Base64-encoded sha256 hash of JSON from reference field. Required if `reference` is included.
        }
    }
}

near_contract_standards::impl_non_fungible_token_core!(Contract, tokens);
near_contract_standards::impl_non_fungible_token_approval!(Contract, tokens);
near_contract_standards::impl_non_fungible_token_enumeration!(Contract, tokens);

fn log_mint(owner_id: &str, token_ids: Vec<String>) {
    NearEvent::log_nft_mint(owner_id.to_string(), token_ids, None);
}

fn on_sale() -> bool {
    cfg!(feature = "on_sale")
}

const fn to_near(num: u32) -> Balance {
    (num as Balance * 10u128.pow(24)) as Balance
}
#[cfg(not(target_arch = "wasm32"))]
#[cfg(test)]
mod tests {
    use super::*;
    const TEN: u128 = to_near(10);
    const ONE: u128 = to_near(1);

    fn new_contract() -> Contract {
        Contract::new_default_meta(
            AccountId::new_unchecked("root".to_string()),
            "name".to_string(),
            "sym".to_string(),
            "https://".to_string(),
            10_000,
            TEN.into(),
            ONE.into(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
    }

    #[test]
    fn check_price() {
        let contract = new_contract();
        assert_eq!(
            contract.cost_per_token(1).0,
            TEN + contract.token_storage_cost().0
        );
        assert_eq!(
            contract.cost_per_token(2).0,
            TEN + contract.token_storage_cost().0 - contract.discount(2).0
        );
        println!(
            "{}, {}, {}",
            contract.discount(1).0,
            contract.discount(2).0,
            contract.discount(10).0
        );
        println!(
            "{}",
            (contract.base_cost - contract.discount(10).0).max(contract.min_cost)
        );
        println!(
            "{}, {}",
            contract.cost_per_token(24).0,
            contract.cost_per_token(10).0
        );
    }
}
