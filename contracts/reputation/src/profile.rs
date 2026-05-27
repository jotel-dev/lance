use soroban_sdk::{contracttype, Address, Bytes};

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ReviewAggregate {
    pub total_points: i128,
    pub reviews: u32,
    pub average_rating_bps: i32,
}

impl ReviewAggregate {
    pub fn new() -> Self {
        Self {
            total_points: 0,
            reviews: 0,
            average_rating_bps: 5_000,
        }
    }
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct RoleMetrics {
    pub score: i32,
    pub completed_jobs: u32,
    pub review: ReviewAggregate,
    pub badge_level: u32,
}

impl RoleMetrics {
    pub fn new() -> Self {
        Self {
            score: 5_000,
            completed_jobs: 0,
            review: ReviewAggregate::new(),
            badge_level: 0,
        }
    }
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub address: Address,
    pub client_score: i32,
    pub client_points: i32,
    pub client_jobs: u32,
    pub client_reviews_weight: u32,
    pub client_badge_level: u32,
    pub freelancer_score: i32,
    pub freelancer_points: i32,
    pub freelancer_jobs: u32,
    pub freelancer_reviews_weight: u32,
    pub freelancer_badge_level: u32,
    pub client: RoleMetrics,
    pub freelancer: RoleMetrics,
    pub is_blacklisted: bool,
    pub metadata_hash: Option<Bytes>,
}

impl Profile {
    pub fn new(address: Address) -> Self {
        Self {
            address,
            client_score: 5000,
            client_points: 0,
            client_jobs: 0,
            client_reviews_weight: 0,
            client_badge_level: 0,
            freelancer_score: 5000,
            freelancer_points: 0,
            freelancer_jobs: 0,
            freelancer_reviews_weight: 0,
            freelancer_badge_level: 0,
            client: RoleMetrics::new(),
            freelancer: RoleMetrics::new(),
            is_blacklisted: false,
            metadata_hash: None,
        }
    }
}
