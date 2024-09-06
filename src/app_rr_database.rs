use deadpool_diesel::pg::{Manager, Pool};
use diesel::{
	sql_types::{BigInt, Binary, Bool, Integer, Nullable, SmallInt, Text},
	PgConnection, RunQueryDsl,
};
use tracing::error;

use crate::{
	app_database::AppDatabaseError, models, ChallengeWithDifficulty, Submission,
	SubmissionWithPubkey, Transaction,
};

pub struct AppRRDatabase {
	connection_pool: Pool,
}

impl AppRRDatabase {
	pub fn new(url: String) -> Self {
		let manager = Manager::new(url, deadpool_diesel::Runtime::Tokio1);

		let pool = Pool::builder(manager).build().unwrap();

		AppRRDatabase { connection_pool: pool }
	}

	pub async fn get_challenge_by_challenge(
		&self,
		challenge: Vec<u8>,
	) -> Result<models::Challenge, AppDatabaseError> {
		if let Ok(db_conn) = self.connection_pool.get().await {
			let res = db_conn.interact(move |conn: &mut PgConnection| {
                diesel::sql_query("SELECT id, pool_id, submission_id, challenge, rewards_earned FROM challenges WHERE challenges.challenge = ?")
                .bind::<Binary, _>(challenge)
                .get_result::<models::Challenge>(conn)
            }).await;

			match res {
				Ok(interaction) => match interaction {
					Ok(query) => {
						return Ok(query);
					},
					Err(e) => {
						error!("{:?}", e);
						return Err(AppDatabaseError::QueryFailed);
					},
				},
				Err(e) => {
					error!("{:?}", e);
					return Err(AppDatabaseError::InteractionFailed);
				},
			}
		} else {
			return Err(AppDatabaseError::FailedToGetConnectionFromPool);
		};
	}

	pub async fn get_miner_rewards(
		&self,
		miner_pubkey: String,
	) -> Result<models::Reward, AppDatabaseError> {
		if let Ok(db_conn) = self.connection_pool.get().await {
			let res = db_conn.interact(move |conn: &mut PgConnection| {
                diesel::sql_query("SELECT r.balance, r.miner_id FROM miners m JOIN rewards r ON m.id = r.miner_id WHERE m.pubkey = ?")
                .bind::<Text, _>(miner_pubkey)
                .get_result::<models::Reward>(conn)
            }).await;

			match res {
				Ok(interaction) => match interaction {
					Ok(query) => {
						return Ok(query);
					},
					Err(e) => {
						error!("get_miner_rewards: {:?}", e);
						return Err(AppDatabaseError::QueryFailed);
					},
				},
				Err(e) => {
					error!("{:?}", e);
					return Err(AppDatabaseError::InteractionFailed);
				},
			}
		} else {
			return Err(AppDatabaseError::FailedToGetConnectionFromPool);
		};
	}

	pub async fn get_last_challenge_submissions(
		&self,
	) -> Result<Vec<SubmissionWithPubkey>, AppDatabaseError> {
		if let Ok(db_conn) = self.connection_pool.get().await {
			let res = db_conn
                .interact(move |conn: &mut PgConnection| {

                    diesel::sql_query("SELECT s.*, m.pubkey FROM submissions s JOIN miners m ON s.miner_id = m.id JOIN challenges c ON s.challenge_id = c.id WHERE c.id = (SELECT id from challenges ORDER BY created DESC LIMIT 1 OFFSET 1)")
                        .load::<SubmissionWithPubkey>(conn)
                })
                .await;

			match res {
				Ok(interaction) => match interaction {
					Ok(query) => {
						return Ok(query);
					},
					Err(e) => {
						error!("{:?}", e);
						return Err(AppDatabaseError::QueryFailed);
					},
				},
				Err(e) => {
					error!("{:?}", e);
					return Err(AppDatabaseError::InteractionFailed);
				},
			}
		} else {
			return Err(AppDatabaseError::FailedToGetConnectionFromPool);
		};
	}

	pub async fn get_miner_earnings(
		&self,
		pubkey: String,
	) -> Result<Vec<Submission>, AppDatabaseError> {
		if let Ok(db_conn) = self.connection_pool.get().await {
			let res = db_conn
                .interact(move |conn: &mut PgConnection| {

                    diesel::sql_query("SELECT s.* FROM submissions s JOIN miners m ON s.miner_id = m.id WHERE m.pubkey = ? ORDER BY s.created DESC LIMIT 100")
                        .bind::<Text, _>(pubkey)
                        .load::<Submission>(conn)
                })
                .await;

			match res {
				Ok(interaction) => match interaction {
					Ok(query) => {
						return Ok(query);
					},
					Err(e) => {
						error!("{:?}", e);
						return Err(AppDatabaseError::QueryFailed);
					},
				},
				Err(e) => {
					error!("{:?}", e);
					return Err(AppDatabaseError::InteractionFailed);
				},
			}
		} else {
			return Err(AppDatabaseError::FailedToGetConnectionFromPool);
		};
	}

	pub async fn get_miner_submissions(
		&self,
		pubkey: String,
	) -> Result<Vec<Submission>, AppDatabaseError> {
		if let Ok(db_conn) = self.connection_pool.get().await {
			let res = db_conn
                .interact(move |conn: &mut PgConnection| {
                    diesel::sql_query("SELECT s.* FROM submissions s JOIN miners m ON s.miner_id = m.id WHERE m.pubkey = ? ORDER BY s.created DESC LIMIT 100")
                        .bind::<Text, _>(pubkey)
                        .load::<Submission>(conn)
                })
                .await;

			match res {
				Ok(interaction) => match interaction {
					Ok(query) => {
						return Ok(query);
					},
					Err(e) => {
						error!("{:?}", e);
						return Err(AppDatabaseError::QueryFailed);
					},
				},
				Err(e) => {
					error!("{:?}", e);
					return Err(AppDatabaseError::InteractionFailed);
				},
			}
		} else {
			return Err(AppDatabaseError::FailedToGetConnectionFromPool);
		};
	}

	pub async fn get_challenges(&self) -> Result<Vec<ChallengeWithDifficulty>, AppDatabaseError> {
		if let Ok(db_conn) = self.connection_pool.get().await {
			let res = db_conn
                .interact(move |conn: &mut PgConnection| {

                    diesel::sql_query("SELECT c.id, c.rewards_earned, c.updated, s.difficulty FROM challenges c JOIN submissions s ON c.submission_id = s.id WHERE c.submission_id IS NOT NULL ORDER BY c.id  DESC LIMIT 1440")
                        .load::<ChallengeWithDifficulty>(conn)
                })
                .await;

			match res {
				Ok(interaction) => match interaction {
					Ok(query) => {
						return Ok(query);
					},
					Err(e) => {
						error!("{:?}", e);
						return Err(AppDatabaseError::QueryFailed);
					},
				},
				Err(e) => {
					error!("{:?}", e);
					return Err(AppDatabaseError::InteractionFailed);
				},
			}
		} else {
			return Err(AppDatabaseError::FailedToGetConnectionFromPool);
		};
	}

	pub async fn get_pool_by_authority_pubkey(
		&self,
		pool_pubkey: String,
	) -> Result<models::Pool, AppDatabaseError> {
		if let Ok(db_conn) = self.connection_pool.get().await {
			let res = db_conn.interact(move |conn: &mut PgConnection| {
                diesel::sql_query("SELECT id, proof_pubkey, authority_pubkey, total_rewards, claimed_rewards FROM pools WHERE pools.authority_pubkey = ?")
                .bind::<Text, _>(pool_pubkey)
                .get_result::<models::Pool>(conn)
            }).await;

			match res {
				Ok(interaction) => match interaction {
					Ok(query) => {
						return Ok(query);
					},
					Err(e) => {
						error!("{:?}", e);
						return Err(AppDatabaseError::QueryFailed);
					},
				},
				Err(e) => {
					error!("{:?}", e);
					return Err(AppDatabaseError::InteractionFailed);
				},
			}
		} else {
			return Err(AppDatabaseError::FailedToGetConnectionFromPool);
		};
	}

	pub async fn get_latest_mine_txn(&self) -> Result<Transaction, AppDatabaseError> {
		if let Ok(db_conn) = self.connection_pool.get().await {
			let res = db_conn
				.interact(move |conn: &mut PgConnection| {
					diesel::sql_query(
						"SELECT * FROM transactions WHERE transaction_type = ? ORDER BY id DESC LIMIT 1",
					)
					.bind::<Text, _>("mine")
					.get_result::<Transaction>(conn)
				})
				.await;

			match res {
				Ok(interaction) => match interaction {
					Ok(query) => {
						return Ok(query);
					},
					Err(e) => {
						error!("{:?}", e);
						return Err(AppDatabaseError::QueryFailed);
					},
				},
				Err(e) => {
					error!("{:?}", e);
					return Err(AppDatabaseError::InteractionFailed);
				},
			}
		} else {
			return Err(AppDatabaseError::FailedToGetConnectionFromPool);
		};
	}
}
