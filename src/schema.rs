// @generated automatically by Diesel CLI.

pub mod ore {
    diesel::table! {
        ore.challenges (id) {
            id -> Int4,
            pool_id -> Int4,
            submission_id -> Nullable<Int4>,
            challenge -> Bytea,
            rewards_earned -> Nullable<Int8>,
            created -> Timestamptz,
            updated -> Timestamptz,
        }
    }

    diesel::table! {
        ore.claims (id) {
            id -> Int4,
            miner_id -> Int4,
            pool_id -> Int4,
            transaction_id -> Int4,
            amount -> Int8,
            created -> Timestamptz,
            updated -> Timestamptz,
        }
    }

    diesel::table! {
        ore.earnings (id) {
            id -> Int4,
            miner_id -> Int4,
            pool_id -> Int4,
            challenge_id -> Int4,
            amount -> Int8,
            created -> Timestamptz,
            updated -> Timestamptz,
        }
    }

    diesel::table! {
        ore.miners (id) {
            id -> Int4,
            #[max_length = 44]
            pubkey -> Varchar,
            enabled -> Bool,
            #[max_length = 30]
            status -> Varchar,
            created -> Timestamptz,
            updated -> Timestamptz,
        }
    }

    diesel::table! {
        ore.pools (id) {
            id -> Int4,
            #[max_length = 44]
            proof_pubkey -> Varchar,
            #[max_length = 44]
            authority_pubkey -> Varchar,
            total_rewards -> Int8,
            claimed_rewards -> Int8,
            created -> Timestamptz,
            updated -> Timestamptz,
        }
    }

    diesel::table! {
        ore.rewards (id) {
            id -> Int4,
            miner_id -> Int4,
            pool_id -> Int4,
            balance -> Int8,
            created -> Timestamptz,
            updated -> Timestamptz,
        }
    }

    diesel::table! {
        ore.submissions (id) {
            id -> Int4,
            miner_id -> Int4,
            challenge_id -> Int4,
            difficulty -> Int2,
            nonce -> Int8,
            digest -> Bytea,
            created -> Timestamptz,
            updated -> Timestamptz,
        }
    }

    diesel::table! {
        ore.transactions (id) {
            id -> Int4,
            #[max_length = 30]
            transaction_type -> Varchar,
            #[max_length = 200]
            signature -> Varchar,
            priority_fee -> Int4,
            pool_id -> Int4,
            miner_id -> Nullable<Int4>,
            created -> Timestamptz,
            updated -> Timestamptz,
        }
    }

    diesel::allow_tables_to_appear_in_same_query!(
        challenges,
        claims,
        earnings,
        miners,
        pools,
        rewards,
        submissions,
        transactions,
    );
}
