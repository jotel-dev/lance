#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, log, panic_with_error, symbol_short,
    token, Address, Bytes, Env, Map, Vec,
};

const MAX_HASH_LEN: u32 = 96;

#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum JobRegistryError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    InvalidJobId = 3,
    InvalidBudget = 4,
    InvalidHash = 5,
    JobAlreadyExists = 6,
    JobNotFound = 7,
    JobNotOpen = 8,
    Unauthorized = 9,
    BidAlreadySubmitted = 10,
    BidNotFound = 11,
    InvalidStateTransition = 12,
    NoDeliverable = 13,
    Overflow = 14,
    BidSubmissionClosed = 15,
    InvalidDeadline = 16,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum JobStatus {
    Open,
    Assigned,
    DeliverableSubmitted,
    Completed,
    Disputed,
}

#[contracttype]
#[derive(Clone)]
pub struct JobRecord {
    pub client: Address,
    pub freelancer: Option<Address>,
    pub metadata_hash: Bytes,
    pub budget_stroops: i128,
    pub status: JobStatus,
    pub bid_deadline: u64,
    pub collateral_token: Address,
    pub collateral_amount: i128,
    pub collateral_locked: bool,
}

#[contracttype]
#[derive(Clone)]
pub struct BidRecord {
    pub freelancer: Address,
    pub proposal_hash: Bytes,
}

#[contracttype]
pub enum DataKey {
    Admin,
    NextJobId,
    Job(u64),
    Bids(u64),
    Deliverable(u64),
}

#[contract]
pub struct JobRegistryContract;

#[contractimpl]
impl JobRegistryContract {
    /// One-time storage bootstrap.
    ///
    /// Sets contract admin and initializes `next_job_id` to 1.
    pub fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic_with_error!(&env, JobRegistryError::AlreadyInitialized);
        }

        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::NextJobId, &1u64);

        log!(&env, "JobRegistry initialized with admin: {}", admin);
        env.events().publish((symbol_short!("init"),), admin);
    }

    /// Returns whether storage has been initialized.
    pub fn is_initialized(env: Env) -> bool {
        env.storage().instance().has(&DataKey::Admin)
    }

    pub fn get_admin(env: Env) -> Address {
        read_admin(&env)
    }

    pub fn get_next_job_id(env: Env) -> u64 {
        read_next_job_id(&env)
    }

    /// Client posts a job with explicit `job_id` and collateral lockup details.
    /// `metadata_hash` is expected to contain CID bytes.
    pub fn post_job(
        env: Env,
        job_id: u64,
        client: Address,
        hash: Bytes,
        budget: i128,
        bid_deadline: u64,
        collateral_token: Address,
        collateral_amount: i128,
    ) {
        ensure_initialized(&env);
        validate_job_input(&env, job_id, &hash, budget);

        client.require_auth();

        let now = env.ledger().timestamp();
        if bid_deadline <= now {
            panic_with_error!(&env, JobRegistryError::InvalidDeadline);
        }
        if collateral_amount < 0 {
            panic_with_error!(&env, JobRegistryError::InvalidBudget);
        }

        post_job_with_id(
            &env,
            job_id,
            client.clone(),
            hash,
            budget,
            bid_deadline,
            collateral_token.clone(),
            collateral_amount,
        );

        // Lock collateral from client into this contract
        if collateral_amount > 0 {
            let token_client = token::Client::new(&env, &collateral_token);
            token_client.transfer(&client, &env.current_contract_address(), &collateral_amount);
        }

        // Keep auto-id monotonic when explicit ids are used.
        let next_job_id = read_next_job_id(&env);
        if job_id >= next_job_id {
            let updated = job_id
                .checked_add(1)
                .unwrap_or_else(|| panic_with_error!(&env, JobRegistryError::Overflow));
            env.storage().instance().set(&DataKey::NextJobId, &updated);
        }

        log!(
            &env,
            "post_job: id {} client {} budget {} deadline {} collateral {} amount {}",
            job_id,
            client,
            budget,
            bid_deadline,
            collateral_token,
            collateral_amount
        );
        env.events()
            .publish((symbol_short!("jobpost"), job_id), (client, budget));
    }

    /// Client posts a job using internal registry index allocation and collateral lockup details.
    pub fn post_job_auto(
        env: Env,
        client: Address,
        hash: Bytes,
        budget: i128,
        bid_deadline: u64,
        collateral_token: Address,
        collateral_amount: i128,
    ) -> u64 {
        ensure_initialized(&env);

        let job_id = read_next_job_id(&env);
        validate_job_input(&env, job_id, &hash, budget);

        client.require_auth();

        let now = env.ledger().timestamp();
        if bid_deadline <= now {
            panic_with_error!(&env, JobRegistryError::InvalidDeadline);
        }
        if collateral_amount < 0 {
            panic_with_error!(&env, JobRegistryError::InvalidBudget);
        }

        post_job_with_id(
            &env,
            job_id,
            client.clone(),
            hash,
            budget,
            bid_deadline,
            collateral_token.clone(),
            collateral_amount,
        );

        // Lock collateral from client into this contract
        if collateral_amount > 0 {
            let token_client = token::Client::new(&env, &collateral_token);
            token_client.transfer(&client, &env.current_contract_address(), &collateral_amount);
        }

        let next = job_id
            .checked_add(1)
            .unwrap_or_else(|| panic_with_error!(&env, JobRegistryError::Overflow));
        env.storage().instance().set(&DataKey::NextJobId, &next);

        log!(
            &env,
            "post_job_auto: id {} client {} budget {} deadline {} collateral {} amount {}",
            job_id,
            client,
            budget,
            bid_deadline,
            collateral_token,
            collateral_amount
        );
        env.events()
            .publish((symbol_short!("jobauto"), job_id), (client, budget));

        job_id
    }

    /// Freelancer submits a bid.
    pub fn submit_bid(env: Env, job_id: u64, freelancer: Address, proposal_hash: Bytes) {
        ensure_initialized(&env);
        validate_hash(&env, &proposal_hash);
        freelancer.require_auth();

        let key = DataKey::Job(job_id);
        let job: JobRecord = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| panic_with_error!(&env, JobRegistryError::JobNotFound));

        if job.status != JobStatus::Open {
            panic_with_error!(&env, JobRegistryError::JobNotOpen);
        }

        let now = env.ledger().timestamp();
        if now > job.bid_deadline {
            panic_with_error!(&env, JobRegistryError::BidSubmissionClosed);
        }

        let bids_key = DataKey::Bids(job_id);
        let mut bids: Map<Address, Bytes> = env
            .storage()
            .persistent()
            .get(&bids_key)
            .unwrap_or(Map::new(&env));

        if bids.contains_key(freelancer.clone()) {
            panic_with_error!(&env, JobRegistryError::BidAlreadySubmitted);
        }

        bids.set(freelancer.clone(), proposal_hash);
        env.storage().persistent().set(&bids_key, &bids);

        log!(&env, "submit_bid: id {} freelancer {}", job_id, freelancer);
        env.events()
            .publish((symbol_short!("bid"), job_id), freelancer);
    }

    /// Client accepts a bid, locking in the freelancer and transitioning state to Assigned.
    pub fn accept_bid(env: Env, job_id: u64, client: Address, freelancer: Address) {
        ensure_initialized(&env);
        client.require_auth();

        let key = DataKey::Job(job_id);
        let mut job: JobRecord = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| panic_with_error!(&env, JobRegistryError::JobNotFound));

        if job.status != JobStatus::Open {
            panic_with_error!(&env, JobRegistryError::JobNotOpen);
        }
        if client != job.client {
            panic_with_error!(&env, JobRegistryError::Unauthorized);
        }

        let bids: Map<Address, Bytes> = env
            .storage()
            .persistent()
            .get(&DataKey::Bids(job_id))
            .unwrap_or(Map::new(&env));

        if !bids.contains_key(freelancer.clone()) {
            panic_with_error!(&env, JobRegistryError::BidNotFound);
        }

        job.freelancer = Some(freelancer.clone());
        job.status = JobStatus::Assigned;
        env.storage().persistent().set(&key, &job);

        log!(
            &env,
            "accept_bid: id {} client {} freelancer {}",
            job_id,
            client,
            freelancer
        );
        env.events()
            .publish((symbol_short!("accept"), job_id), freelancer);
    }

    /// Freelancer submits deliverable IPFS hash.
    pub fn submit_deliverable(env: Env, job_id: u64, freelancer: Address, hash: Bytes) {
        ensure_initialized(&env);
        validate_hash(&env, &hash);
        freelancer.require_auth();

        let key = DataKey::Job(job_id);
        let mut job: JobRecord = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| panic_with_error!(&env, JobRegistryError::JobNotFound));

        if job.status != JobStatus::Assigned {
            panic_with_error!(&env, JobRegistryError::InvalidStateTransition);
        }
        if job.freelancer != Some(freelancer.clone()) {
            panic_with_error!(&env, JobRegistryError::Unauthorized);
        }

        job.status = JobStatus::DeliverableSubmitted;
        env.storage().persistent().set(&key, &job);
        env.storage()
            .persistent()
            .set(&DataKey::Deliverable(job_id), &hash);

        log!(
            &env,
            "submit_deliverable: id {} freelancer {}",
            job_id,
            freelancer
        );
        env.events()
            .publish((symbol_short!("deliver"), job_id), freelancer);
    }

    /// Client completes a job, releasing locked collateral to the freelancer.
    pub fn complete_job(env: Env, job_id: u64, client: Address) {
        ensure_initialized(&env);
        client.require_auth();

        let key = DataKey::Job(job_id);
        let mut job: JobRecord = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| panic_with_error!(&env, JobRegistryError::JobNotFound));

        if client != job.client {
            panic_with_error!(&env, JobRegistryError::Unauthorized);
        }

        if job.status != JobStatus::DeliverableSubmitted {
            panic_with_error!(&env, JobRegistryError::InvalidStateTransition);
        }

        job.status = JobStatus::Completed;

        if job.collateral_locked && job.collateral_amount > 0 {
            if let Some(ref freelancer) = job.freelancer {
                let token_client = token::Client::new(&env, &job.collateral_token);
                token_client.transfer(
                    &env.current_contract_address(),
                    freelancer,
                    &job.collateral_amount,
                );
                job.collateral_locked = false;
            }
        }

        env.storage().persistent().set(&key, &job);

        log!(&env, "complete_job: id {}", job_id);
        env.events().publish((symbol_short!("complete"), job_id), ());
    }

    /// Client refunds their locked collateral if the job has expired without an accepted bid.
    pub fn refund_collateral(env: Env, job_id: u64, client: Address) {
        ensure_initialized(&env);
        client.require_auth();

        let key = DataKey::Job(job_id);
        let mut job: JobRecord = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| panic_with_error!(&env, JobRegistryError::JobNotFound));

        if client != job.client {
            panic_with_error!(&env, JobRegistryError::Unauthorized);
        }

        // Refund is only allowed if status is Open and the bid deadline has passed
        let now = env.ledger().timestamp();
        if job.status != JobStatus::Open || now <= job.bid_deadline {
            panic_with_error!(&env, JobRegistryError::InvalidStateTransition);
        }

        if job.collateral_locked && job.collateral_amount > 0 {
            let token_client = token::Client::new(&env, &job.collateral_token);
            token_client.transfer(
                &env.current_contract_address(),
                &job.client,
                &job.collateral_amount,
            );
            job.collateral_locked = false;
        }

        env.storage().persistent().set(&key, &job);

        log!(&env, "refund_collateral: id {}", job_id);
        env.events().publish((symbol_short!("refund"), job_id), ());
    }

    /// Mark job disputed. Only the initialized admin can call this.
    pub fn mark_disputed(env: Env, job_id: u64) {
        ensure_initialized(&env);
        let admin = read_admin(&env);
        admin.require_auth();

        let key = DataKey::Job(job_id);
        let mut job: JobRecord = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| panic_with_error!(&env, JobRegistryError::JobNotFound));

        if job.status != JobStatus::Assigned && job.status != JobStatus::DeliverableSubmitted {
            panic_with_error!(&env, JobRegistryError::InvalidStateTransition);
        }

        job.status = JobStatus::Disputed;
        env.storage().persistent().set(&key, &job);

        log!(&env, "mark_disputed: id {}", job_id);
        env.events().publish((symbol_short!("dispute"), job_id), ());
    }

    pub fn get_job(env: Env, job_id: u64) -> JobRecord {
        ensure_initialized(&env);
        env.storage()
            .persistent()
            .get(&DataKey::Job(job_id))
            .unwrap_or_else(|| panic_with_error!(&env, JobRegistryError::JobNotFound))
    }

    pub fn get_bids(env: Env, job_id: u64) -> Vec<BidRecord> {
        ensure_initialized(&env);
        let bids_map: Map<Address, Bytes> = env
            .storage()
            .persistent()
            .get(&DataKey::Bids(job_id))
            .unwrap_or(Map::new(&env));

        let mut bids_vec = Vec::new(&env);
        for (freelancer, proposal_hash) in bids_map.iter() {
            bids_vec.push_back(BidRecord {
                freelancer,
                proposal_hash,
            });
        }
        bids_vec
    }

    pub fn get_deliverable(env: Env, job_id: u64) -> Bytes {
        ensure_initialized(&env);
        env.storage()
            .persistent()
            .get(&DataKey::Deliverable(job_id))
            .unwrap_or_else(|| panic_with_error!(&env, JobRegistryError::NoDeliverable))
    }
}

fn ensure_initialized(env: &Env) {
    if !env.storage().instance().has(&DataKey::Admin) {
        panic_with_error!(env, JobRegistryError::NotInitialized);
    }
}

fn read_admin(env: &Env) -> Address {
    ensure_initialized(env);
    env.storage()
        .instance()
        .get(&DataKey::Admin)
        .unwrap_or_else(|| panic_with_error!(env, JobRegistryError::NotInitialized))
}

fn read_next_job_id(env: &Env) -> u64 {
    ensure_initialized(env);
    env.storage()
        .instance()
        .get(&DataKey::NextJobId)
        .unwrap_or_else(|| panic_with_error!(env, JobRegistryError::NotInitialized))
}

fn validate_job_input(env: &Env, job_id: u64, hash: &Bytes, budget: i128) {
    if job_id == 0 {
        panic_with_error!(env, JobRegistryError::InvalidJobId);
    }
    if budget <= 0 {
        panic_with_error!(env, JobRegistryError::InvalidBudget);
    }
    validate_hash(env, hash);
}

fn validate_hash(env: &Env, hash: &Bytes) {
    let len = hash.len();
    if len == 0 || len > MAX_HASH_LEN {
        panic_with_error!(env, JobRegistryError::InvalidHash);
    }
}

fn post_job_with_id(
    env: &Env,
    job_id: u64,
    client: Address,
    hash: Bytes,
    budget: i128,
    bid_deadline: u64,
    collateral_token: Address,
    collateral_amount: i128,
) {
    let key = DataKey::Job(job_id);
    if env.storage().persistent().has(&key) {
        panic_with_error!(env, JobRegistryError::JobAlreadyExists);
    }

    let job = JobRecord {
        client,
        freelancer: None,
        metadata_hash: hash,
        budget_stroops: budget,
        status: JobStatus::Open,
        bid_deadline,
        collateral_token,
        collateral_amount,
        collateral_locked: collateral_amount > 0,
    };
    env.storage().persistent().set(&key, &job);

    let bids: Map<Address, Bytes> = Map::new(env);
    env.storage()
        .persistent()
        .set(&DataKey::Bids(job_id), &bids);
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger as _};
    use soroban_sdk::{Address, Bytes, Env};

    fn setup() -> (
        Env,
        JobRegistryContractClient<'static>,
        Address,
        Address,
        Address,
        Address, // Mock Token
    ) {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let client = Address::generate(&env);
        let freelancer = Address::generate(&env);

        let token_addr = env.register_stellar_asset_contract_v2(admin.clone()).address();
        let token_client = token::StellarAssetClient::new(&env, &token_addr);
        token_client.mint(&client, &100_000);

        let contract_id = env.register_contract(None, JobRegistryContract);
        let cc = JobRegistryContractClient::new(&env, &contract_id);

        (env, cc, admin, client, freelancer, token_addr)
    }

    #[test]
    fn test_initialize_bootstraps_storage() {
        let (_env, cc, admin, _, _, _) = setup();

        cc.initialize(&admin);

        assert!(cc.is_initialized());
        assert_eq!(cc.get_admin(), admin);
        assert_eq!(cc.get_next_job_id(), 1u64);
    }

    #[test]
    #[should_panic]
    fn test_double_initialize_panics() {
        let (_env, cc, admin, _, _, _) = setup();

        cc.initialize(&admin);
        cc.initialize(&admin);
    }

    #[test]
    #[should_panic]
    fn test_post_job_before_initialize_panics() {
        let (env, cc, _admin, client, _, token_addr) = setup();
        let hash = Bytes::from_slice(&env, b"QmHash");
        cc.post_job(&1u64, &client, &hash, &5000i128, &2000u64, &token_addr, &1000i128);
    }

    #[test]
    fn test_post_job_auto_allocates_sequential_ids() {
        let (env, cc, admin, client, _, token_addr) = setup();
        cc.initialize(&admin);

        let hash1 = Bytes::from_slice(&env, b"QmHash1");
        let hash2 = Bytes::from_slice(&env, b"QmHash2");

        env.ledger().set_timestamp(100);

        let id1 = cc.post_job_auto(&client, &hash1, &5000i128, &1000u64, &token_addr, &1000i128);
        let id2 = cc.post_job_auto(&client, &hash2, &7000i128, &2000u64, &token_addr, &2000i128);

        assert_eq!(id1, 1u64);
        assert_eq!(id2, 2u64);
        assert_eq!(cc.get_next_job_id(), 3u64);
    }

    #[test]
    fn test_post_job_with_explicit_id_updates_next_job_id() {
        let (env, cc, admin, client, _, token_addr) = setup();
        cc.initialize(&admin);

        let hash = Bytes::from_slice(&env, b"QmHash");
        env.ledger().set_timestamp(100);
        cc.post_job(&42u64, &client, &hash, &5000i128, &1000u64, &token_addr, &1000i128);

        assert_eq!(cc.get_next_job_id(), 43u64);
    }

    #[test]
    #[should_panic]
    fn test_invalid_budget_panics() {
        let (env, cc, admin, client, _, token_addr) = setup();
        cc.initialize(&admin);

        let hash = Bytes::from_slice(&env, b"QmHash");
        env.ledger().set_timestamp(100);
        cc.post_job(&1u64, &client, &hash, &0i128, &1000u64, &token_addr, &1000i128);
    }

    #[test]
    #[should_panic]
    fn test_empty_hash_panics() {
        let (env, cc, admin, client, _, token_addr) = setup();
        cc.initialize(&admin);

        let empty = Bytes::from_slice(&env, b"");
        env.ledger().set_timestamp(100);
        cc.post_job(&1u64, &client, &empty, &5000i128, &1000u64, &token_addr, &1000i128);
    }

    #[test]
    fn test_full_lifecycle() {
        let (env, cc, admin, client, freelancer, token_addr) = setup();
        cc.initialize(&admin);

        let hash = Bytes::from_slice(&env, b"QmSomeIPFSHash");
        env.ledger().set_timestamp(100);
        cc.post_job(&1u64, &client, &hash, &5000i128, &1000u64, &token_addr, &1000i128);

        let tc = token::Client::new(&env, &token_addr);
        assert_eq!(tc.balance(&cc.address), 1000);

        let job = cc.get_job(&1u64);
        assert_eq!(job.status, JobStatus::Open);
        assert_eq!(job.freelancer, None);
        assert!(job.collateral_locked);

        let proposal = Bytes::from_slice(&env, b"QmProposalHash");
        cc.submit_bid(&1u64, &freelancer, &proposal);

        let bids = cc.get_bids(&1u64);
        assert_eq!(bids.len(), 1);

        cc.accept_bid(&1u64, &client, &freelancer);
        let job = cc.get_job(&1u64);
        assert_eq!(job.status, JobStatus::Assigned);
        assert_eq!(job.freelancer, Some(freelancer.clone()));

        let deliverable = Bytes::from_slice(&env, b"QmDeliverableHash");
        cc.submit_deliverable(&1u64, &freelancer, &deliverable);

        let job = cc.get_job(&1u64);
        assert_eq!(job.status, JobStatus::DeliverableSubmitted);

        let d = cc.get_deliverable(&1u64);
        assert_eq!(d, deliverable);

        cc.complete_job(&1u64, &client);
        let job = cc.get_job(&1u64);
        assert_eq!(job.status, JobStatus::Completed);
        assert!(!job.collateral_locked);
        assert_eq!(tc.balance(&freelancer), 1000);
    }

    #[test]
    #[should_panic]
    fn test_duplicate_bid_panics() {
        let (env, cc, admin, client, freelancer, token_addr) = setup();
        cc.initialize(&admin);

        let hash = Bytes::from_slice(&env, b"QmHash");
        env.ledger().set_timestamp(100);
        cc.post_job(&1u64, &client, &hash, &5000i128, &1000u64, &token_addr, &1000i128);

        let proposal = Bytes::from_slice(&env, b"QmProposal");
        cc.submit_bid(&1u64, &freelancer, &proposal);
        cc.submit_bid(&1u64, &freelancer, &proposal);
    }

    #[test]
    #[should_panic]
    fn test_accept_without_matching_bid_panics() {
        let (env, cc, admin, client, freelancer, token_addr) = setup();
        cc.initialize(&admin);

        let hash = Bytes::from_slice(&env, b"QmHash");
        env.ledger().set_timestamp(100);
        cc.post_job(&1u64, &client, &hash, &5000i128, &1000u64, &token_addr, &1000i128);

        cc.accept_bid(&1u64, &client, &freelancer);
    }

    #[test]
    fn test_mark_disputed_from_assigned() {
        let (env, cc, admin, client, freelancer, token_addr) = setup();
        cc.initialize(&admin);

        let hash = Bytes::from_slice(&env, b"QmHash");
        env.ledger().set_timestamp(100);
        cc.post_job(&1u64, &client, &hash, &5000i128, &1000u64, &token_addr, &1000i128);

        let proposal = Bytes::from_slice(&env, b"QmProposal");
        cc.submit_bid(&1u64, &freelancer, &proposal);
        cc.accept_bid(&1u64, &client, &freelancer);

        cc.mark_disputed(&1u64);
        let job = cc.get_job(&1u64);
        assert_eq!(job.status, JobStatus::Disputed);
    }

    #[test]
    #[should_panic]
    fn test_mark_disputed_from_open_panics() {
        let (env, cc, admin, client, _, token_addr) = setup();
        cc.initialize(&admin);

        let hash = Bytes::from_slice(&env, b"QmHash");
        env.ledger().set_timestamp(100);
        cc.post_job(&1u64, &client, &hash, &5000i128, &1000u64, &token_addr, &1000i128);

        cc.mark_disputed(&1u64);
    }

    #[test]
    #[should_panic]
    fn test_get_deliverable_without_submission_panics() {
        let (env, cc, admin, client, _, token_addr) = setup();
        cc.initialize(&admin);

        let hash = Bytes::from_slice(&env, b"QmHash");
        env.ledger().set_timestamp(100);
        cc.post_job(&1u64, &client, &hash, &5000i128, &1000u64, &token_addr, &1000i128);

        cc.get_deliverable(&1u64);
    }

    #[test]
    #[should_panic]
    fn test_late_bid_submission_panics() {
        let (env, cc, admin, client, freelancer, token_addr) = setup();
        cc.initialize(&admin);

        let hash = Bytes::from_slice(&env, b"QmHash");
        env.ledger().set_timestamp(100);
        cc.post_job(&1u64, &client, &hash, &5000i128, &1000u64, &token_addr, &1000i128);

        env.ledger().set_timestamp(1001); // past the deadline of 1000
        let proposal = Bytes::from_slice(&env, b"QmProposal");
        cc.submit_bid(&1u64, &freelancer, &proposal);
    }

    #[test]
    fn test_refund_collateral_after_deadline() {
        let (env, cc, admin, client, _, token_addr) = setup();
        cc.initialize(&admin);

        let hash = Bytes::from_slice(&env, b"QmHash");
        env.ledger().set_timestamp(100);
        cc.post_job(&1u64, &client, &hash, &5000i128, &1000u64, &token_addr, &1000i128);

        let tc = token::Client::new(&env, &token_addr);
        assert_eq!(tc.balance(&client), 99000);

        env.ledger().set_timestamp(1001); // past the deadline of 1000
        cc.refund_collateral(&1u64, &client);

        let job = cc.get_job(&1u64);
        assert!(!job.collateral_locked);
        assert_eq!(tc.balance(&client), 100000);
    }
}
