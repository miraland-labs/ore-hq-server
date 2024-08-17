use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    ops::{ControlFlow, Div, Range},
    path::Path,
    str::FromStr,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use ::ore_utils::AccountDeserialize;
use dynamic_fee as pfee;

use axum::{
    debug_handler,
    extract::{
        ws::{Message, WebSocket},
        ConnectInfo, Query, State, WebSocketUpgrade,
    },
    http::{Response, StatusCode},
    response::IntoResponse,
    routing::get,
    Extension, Router,
};
use axum_extra::{headers::authorization::Basic, TypedHeader};
use base64::{prelude::BASE64_STANDARD, Engine};
use clap::{
    builder::{
        styling::{AnsiColor, Effects},
        Styles,
    },
    command, Parser,
};
use drillx::Solution;
use futures::{stream::SplitSink, SinkExt, StreamExt};
use ore_api::{consts::BUS_COUNT, error::OreError, state::Proof};
use rand::Rng;
use serde::Deserialize;
use solana_account_decoder::UiAccountEncoding;
use solana_client::{
    client_error::ClientErrorKind,
    nonblocking::{pubsub_client::PubsubClient, rpc_client::RpcClient},
    rpc_config::{RpcAccountInfoConfig, RpcSendTransactionConfig},
};
use solana_sdk::{
    commitment_config::{CommitmentConfig, CommitmentLevel},
    compute_budget::ComputeBudgetInstruction,
    native_token::LAMPORTS_PER_SOL,
    pubkey::Pubkey,
    signature::{read_keypair_file, Keypair, Signature},
    signer::Signer,
    transaction::Transaction,
};
use solana_transaction_status::UiTransactionEncoding;
use utils::{
    get_auth_ix, get_cutoff, get_mine_ix, get_proof, get_proof_and_best_bus, get_register_ix,
    proof_pubkey, ORE_TOKEN_DECIMALS,
};
// use spl_associated_token_account::get_associated_token_address;
use tokio::sync::{
    mpsc::{UnboundedReceiver, UnboundedSender},
    Mutex, RwLock,
};
use tower_http::trace::{DefaultMakeSpan, TraceLayer};
use tracing::{error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod dynamic_fee;
mod utils;

// MI
// min hash power is matching with ore BASE_REWARD_RATE_MIN_THRESHOLD
// min difficulty, matching with MIN_HASHPOWER.
const MIN_HASHPOWER: u64 = 5;
const MIN_DIFF: u32 = 5;

const RPC_RETRIES: usize = 0;
// const CONFIRM_RETRIES: usize = 8;
// const CONFIRM_DELAY: u64 = 500;

struct AppState {
    sockets: HashMap<SocketAddr, (Pubkey, Arc<Mutex<SplitSink<WebSocket, Message>>>)>,
    miner_sockets: HashMap<Pubkey, Arc<Mutex<SplitSink<WebSocket, Message>>>>,
}

pub struct MessageInternalMineSuccess {
    difficulty: u32,
    total_balance: f64,
    rewards: u64,
    total_hashpower: u64,
    submissions: HashMap<Pubkey, (u32, u64)>,
}

#[derive(Debug)]
pub enum ClientMessage {
    Ready(SocketAddr),
    Mining(SocketAddr),
    BestSolution(SocketAddr, Solution, Pubkey),
}

pub struct EpochHashes {
    best_hash: BestHash,
    submissions: HashMap<Pubkey, (/* diff: */ u32, /* hashpower: */ u64)>,
}

pub struct BestHash {
    solution: Option<Solution>,
    difficulty: u32,
}

pub struct DifficultyPayload {
    pub solution_difficulty: u32,
    pub expected_min_difficulty: u32,
    pub extra_fee_difficulty: u32,
    pub extra_fee_percent: u64,
}

#[derive(Parser, Debug)]
#[command(version, author, about, long_about = None, styles = styles())]
struct Args {
    #[arg(
        long,
        short,
        value_name = "BUFFER_SECONDS",
        help = "The number seconds before the deadline to stop mining and start submitting.",
        default_value = "5"
    )]
    pub buffer_time: u64,

    #[arg(
        long,
        value_name = "FEE_MICROLAMPORTS",
        help = "Price to pay for compute units when dynamic fee flag is off, or dynamic fee is unavailable.",
        default_value = "20000",
        global = true
    )]
    priority_fee: Option<u64>,

    #[arg(
        long,
        value_name = "FEE_CAP_MICROLAMPORTS",
        help = "Max price to pay for compute units when dynamic fees are enabled.",
        default_value = "500000",
        global = true
    )]
    priority_fee_cap: Option<u64>,

    #[arg(long, help = "Enable dynamic priority fees", global = true)]
    dynamic_fee: bool,

    #[arg(
        long,
        value_name = "DYNAMIC_FEE_URL",
        help = "RPC URL to use for dynamic fee estimation.",
        global = true
    )]
    dynamic_fee_url: Option<String>,

    #[arg(
        long,
        short,
        value_name = "EXPECTED_MIN_DIFFICULTY",
        help = "The expected min difficulty to submit for miner.",
        default_value = "18"
    )]
    pub expected_min_difficulty: u32,

    #[arg(
        long,
        short,
        value_name = "EXTRA_FEE_DIFFICULTY",
        help = "The min difficulty that the miner thinks deserves to pay more priority fee.",
        default_value = "27"
    )]
    pub extra_fee_difficulty: u32,

    #[arg(
        long,
        short,
        value_name = "EXTRA_FEE_PERCENT",
        help = "The extra percentage that the miner feels deserves to pay more of the priority fee. A positive integer in the range 0..100 [inclusive] is preferred (although integer > 100 is possible, but not recommended), and the final priority fee cannot exceed the priority fee cap.",
        default_value = "0"
    )]
    pub extra_fee_percent: u64,

    /// Mine with sound notification on/off
    #[arg(
        long,
        value_name = "NO_SOUND_NOTIFICATION",
        help = "Sound notification off by default",
        default_value = "false",
        global = true
    )]
    pub no_sound_notification: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    color_eyre::install().unwrap();
    dotenv::dotenv().ok();
    let args = Args::parse();
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ore_hq_server=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // load envs
    let wallet_path_str = std::env::var("WALLET_PATH").expect("WALLET_PATH must be set.");
    let rpc_url = std::env::var("RPC_URL").expect("RPC_URL must be set.");
    let rpc_ws_url = std::env::var("RPC_WS_URL").expect("RPC_WS_URL must be set.");

    // let virtual_database: Arc<DashMap<Pubkey, Submission>> = DashMap::new().into();

    let priority_fee = Arc::new(args.priority_fee);
    let priority_fee_cap = Arc::new(args.priority_fee_cap);

    let buffer_time = Arc::new(args.buffer_time);

    let min_difficulty = Arc::new(args.expected_min_difficulty);
    let extra_fee_difficulty = Arc::new(args.extra_fee_difficulty);
    let extra_fee_percent = Arc::new(args.extra_fee_percent);

    let dynamic_fee = Arc::new(args.dynamic_fee);
    let dynamic_fee_url = Arc::new(args.dynamic_fee_url);

    let no_sound_notification = Arc::new(args.no_sound_notification);

    // load wallet
    let wallet_path = Path::new(&wallet_path_str);

    if !wallet_path.exists() {
        tracing::error!("Failed to load wallet at: {}", wallet_path_str);
        return Err("Failed to find wallet path.".into());
    }

    let wallet = read_keypair_file(wallet_path)
        .expect("Failed to load keypair from file: {wallet_path_str}");
    println!("loaded wallet {}", wallet.pubkey().to_string());

    println!("establishing rpc connection...");
    let rpc_client = RpcClient::new_with_commitment(rpc_url, CommitmentConfig::confirmed());

    println!("loading sol balance...");
    let balance = if let Ok(balance) = rpc_client.get_balance(&wallet.pubkey()).await {
        balance
    } else {
        return Err("Failed to load balance".into());
    };

    info!("Balance: {:.2}", balance as f64 / LAMPORTS_PER_SOL as f64);

    if balance < 1_000_000 {
        return Err("Sol balance is too low!".into());
    }

    let proof = if let Ok(loaded_proof) = get_proof(&rpc_client, wallet.pubkey()).await {
        loaded_proof
    } else {
        info!("Failed to load proof.");
        info!("Creating proof account...");

        let ix = get_register_ix(wallet.pubkey());

        if let Ok((hash, _slot)) = rpc_client
            .get_latest_blockhash_with_commitment(rpc_client.commitment())
            .await
        {
            let mut tx = Transaction::new_with_payer(&[ix], Some(&wallet.pubkey()));

            tx.sign(&[&wallet], hash);

            let result = rpc_client
                .send_and_confirm_transaction_with_spinner_and_commitment(
                    &tx,
                    rpc_client.commitment(),
                )
                .await;

            if let Ok(sig) = result {
                println!("Sig: {}", sig.to_string());
            } else {
                return Err("Failed to create proof account".into());
            }
        }
        let proof = if let Ok(loaded_proof) = get_proof(&rpc_client, wallet.pubkey()).await {
            loaded_proof
        } else {
            return Err("Failed to get newly created proof".into());
        };
        proof
    };

    let epoch_hashes = Arc::new(RwLock::new(EpochHashes {
        best_hash: BestHash {
            solution: None,
            difficulty: 0,
        },
        submissions: HashMap::new(),
    }));

    let wallet_extension = Arc::new(wallet);
    let proof_ext = Arc::new(Mutex::new(proof));
    let nonce_ext = Arc::new(Mutex::new(0u64));

    let client_nonce_ranges = Arc::new(RwLock::new(HashMap::new()));

    let shared_state = Arc::new(RwLock::new(AppState {
        sockets: HashMap::new(),
        miner_sockets: HashMap::new(),
    }));
    let ready_clients = Arc::new(Mutex::new(HashSet::new()));

    let app_wallet = wallet_extension.clone();
    let app_proof = proof_ext.clone();
    // Establish webocket connection for tracking pool proof changes.
    tokio::spawn(async move {
        proof_tracking_system(rpc_ws_url, app_wallet, app_proof).await;
    });

    let (client_message_sender, client_message_receiver) =
        tokio::sync::mpsc::unbounded_channel::<ClientMessage>();

    // Handle client messages
    let app_shared_state = shared_state.clone();
    let app_ready_clients = ready_clients.clone();
    let app_proof = proof_ext.clone();
    let app_epoch_hashes = epoch_hashes.clone();
    let app_client_nonce_ranges = client_nonce_ranges.clone();
    tokio::spawn(async move {
        client_message_handler_system(
            client_message_receiver,
            &app_shared_state,
            app_epoch_hashes,
            app_ready_clients,
            app_proof,
            app_client_nonce_ranges,
            *min_difficulty,
        )
        .await;
    });

    // Handle ready clients
    let app_shared_state = shared_state.clone();
    let app_proof = proof_ext.clone();
    let app_nonce = nonce_ext.clone();
    let app_epoch_hashes = epoch_hashes.clone();

    let app_client_nonce_ranges = client_nonce_ranges.clone();

    tokio::spawn(async move {
        loop {
            let mut clients = Vec::new();
            {
                let ready_clients_lock = ready_clients.lock().await;
                for ready_client in ready_clients_lock.iter() {
                    clients.push(ready_client.clone());
                }
            };

            let current_proof = { app_proof.lock().await.clone() };

            let cutoff = get_cutoff(proof, *buffer_time);
            let mut should_mine = true;
            let cutoff = if cutoff <= 0 {
                let solution = app_epoch_hashes.read().await.best_hash.solution;
                if solution.is_some() {
                    should_mine = false;
                }
                0
            } else {
                cutoff
            };

            if should_mine {
                let challenge = current_proof.challenge;

                for client in clients {
                    let nonce_range = {
                        let mut nonce = app_nonce.lock().await;
                        let start = *nonce;
                        // max hashes possible in 60s for a single client
                        *nonce += 2_000_000;
                        let end = *nonce;
                        start..end
                    };
                    {
                        let shared_state = app_shared_state.read().await;
                        // message type is 8 bits = 1 u8
                        // challenge is 256 bits = 32 u8
                        // cutoff is 64 bits = 8 u8
                        // nonce_range is 128 bits, start is 64 bits, end is 64 bits = 16 u8
                        let mut bin_data = [0; 57];
                        bin_data[00..1].copy_from_slice(&0u8.to_le_bytes());
                        bin_data[01..33].copy_from_slice(&challenge);
                        bin_data[33..41].copy_from_slice(&cutoff.to_le_bytes());
                        bin_data[41..49].copy_from_slice(&nonce_range.start.to_le_bytes());
                        bin_data[49..57].copy_from_slice(&nonce_range.end.to_le_bytes());

                        if let Some(sender) = shared_state.sockets.get(&client) {
                            let _ = sender
                                .1
                                .lock()
                                .await
                                .send(Message::Binary(bin_data.to_vec()))
                                .await;
                            let _ = ready_clients.lock().await.remove(&client);
                            let _ = app_client_nonce_ranges
                                .write()
                                .await
                                .insert(sender.0, nonce_range);
                        }
                    }
                }
            }

            tokio::time::sleep(Duration::from_secs(*buffer_time)).await;
        }
    });

    let (mine_success_sender, mut mine_success_receiver) =
        tokio::sync::mpsc::unbounded_channel::<MessageInternalMineSuccess>();

    let rpc_client = Arc::new(rpc_client);
    let app_proof = proof_ext.clone();
    let app_epoch_hashes = epoch_hashes.clone();
    let app_wallet = wallet_extension.clone();
    let app_nonce = nonce_ext.clone();
    let app_rpc_client = rpc_client.clone();
    tokio::spawn(async move {
        let rpc_client = app_rpc_client;
        loop {
            let mut old_proof = { app_proof.lock().await.clone() };

            let cutoff = get_cutoff(old_proof, 0);
            if cutoff <= 0 {
                // process solutions
                let solution = { app_epoch_hashes.read().await.best_hash.solution.clone() };
                if let Some(solution) = solution {
                    let signer = app_wallet.clone();
                    let mut ixs = vec![];

                    // TODO: choose better cu limit
                    let cu_limit_ix = ComputeBudgetInstruction::set_compute_unit_limit(480000);
                    ixs.push(cu_limit_ix);

                    let mut fee_type: &str = "static";
                    let fee: u64 = if *dynamic_fee {
                        fee_type = "estimate";
                        match pfee::dynamic_fee(
                            &rpc_client,
                            (*dynamic_fee_url).clone(),
                            *priority_fee_cap,
                        )
                        .await
                        {
                            Ok(fee) => {
                                let mut prio_fee = fee;
                                // MI: calc uplimit of priority fee for precious fee difficulty, eg. diff > 27
                                {
                                    let solution_difficulty = solution.to_hash().difficulty();
                                    if solution_difficulty > *extra_fee_difficulty {
                                        prio_fee = if let Some(ref priority_fee_cap) =
                                            *priority_fee_cap
                                        {
                                            (*priority_fee_cap).min(
                                                prio_fee
                                                    .saturating_mul(
                                                        100u64.saturating_add(*extra_fee_percent),
                                                    )
                                                    .saturating_div(100),
                                            )
                                        } else {
                                            // No priority_fee set as cap, not exceed 300K
                                            300_000.min(
                                                prio_fee
                                                    .saturating_mul(
                                                        100u64.saturating_add(*extra_fee_percent),
                                                    )
                                                    .saturating_div(100),
                                            )
                                        }
                                    }
                                }
                                prio_fee
                            }
                            Err(err) => {
                                let fee = priority_fee.unwrap_or(0);
                                println!(
                                    "Error: {} Falling back to static value: {} microlamports",
                                    err, fee
                                );
                                fee
                            }
                        }
                    } else {
                        // static
                        priority_fee.unwrap_or(0)
                    };

                    let prio_fee_ix = ComputeBudgetInstruction::set_compute_unit_price(fee);
                    ixs.push(prio_fee_ix);

                    let noop_ix = get_auth_ix(signer.pubkey());
                    ixs.push(noop_ix);

                    let mut bus = rand::thread_rng().gen_range(0..BUS_COUNT);
                    if let Ok((_l_proof, (best_bus_id, _best_bus))) =
                        get_proof_and_best_bus(&rpc_client, signer.pubkey()).await
                    {
                        let _now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .expect("Time went backwards")
                            .as_secs();

                        // proof = _l_proof;
                        bus = best_bus_id;
                    }

                    let ix_mine = get_mine_ix(signer.pubkey(), solution, bus);
                    ixs.push(ix_mine);

                    let difficulty = solution.to_hash().difficulty();
                    info!(
                        "Starting mine submission attempts with difficulty {}.",
                        difficulty
                    );

                    let send_cfg = RpcSendTransactionConfig {
                        skip_preflight: true,
                        preflight_commitment: Some(CommitmentLevel::Confirmed),
                        encoding: Some(UiTransactionEncoding::Base64),
                        max_retries: Some(RPC_RETRIES),
                        min_context_slot: None,
                    };

                    for i in 0..5 {
                        if let Ok((hash, _slot)) = rpc_client
                            .get_latest_blockhash_with_commitment(rpc_client.commitment())
                            .await
                        {
                            let mut tx = Transaction::new_with_payer(&ixs, Some(&signer.pubkey()));

                            tx.sign(&[&signer], hash);
                            info!(
                                "Sending signed tx... with {} priority fee {}",
                                fee_type, fee
                            );
                            info!("attempt: {}", i + 1);

                            // let sig = rpc_client
                            //     .send_and_confirm_transaction_with_spinner(&tx)
                            //     .await;
                            // if let Ok(sig) = sig {
                            //     info!("Success!!");
                            //     info!("Sig: {}", sig);
                            //     if ! *no_sound_notification.clone() {
                            //         utils::play_sound();
                            //     }

                            //     // update proof
                            //     // limit number of checking no more than 10
                            //     let mut num_checking = 0;
                            //     loop {
                            //         info!("Waiting for proof hash update");
                            //         let latest_proof = { app_proof.lock().await.clone() };

                            //         if old_proof.challenge.eq(&latest_proof.challenge) {
                            //             info!("Proof challenge not updated yet..");
                            //             old_proof = latest_proof;
                            //             tokio::time::sleep(Duration::from_millis(1000)).await;
                            //             num_checking += 1;
                            //             if num_checking < 10 {
                            //                 continue;
                            //             } else {
                            //                 info!("No proof hash update detected after 10 checkpoints. No more waiting, just keep going...");
                            //                 break;
                            //             }
                            //             // MI
                            //             // continue;
                            //         } else {
                            //             info!("Proof challenge updated! Checking rewards earned.");
                            //             let balance = (latest_proof.balance as f64)
                            //                 / 10f64.powf(ORE_TOKEN_DECIMALS as f64);
                            //             info!("New balance: {}", balance);
                            //             let rewards = latest_proof.balance - old_proof.balance;
                            //             let dec_rewards = (rewards as f64)
                            //                 / 10f64.powf(ORE_TOKEN_DECIMALS as f64);
                            //             info!("Earned: {} ORE", dec_rewards);

                            //             let submissions =
                            //                 { app_epoch_hashes.read().await.submissions.clone() };

                            //             let mut total_hashpower: u64 = 0;

                            //             for submission in submissions.iter() {
                            //                 total_hashpower += submission.1 .1
                            //             }

                            //             let _ =
                            //                 mine_success_sender.send(MessageInternalMineSuccess {
                            //                     difficulty,
                            //                     total_balance: balance,
                            //                     rewards,
                            //                     total_hashpower,
                            //                     submissions,
                            //                 });

                            //             {
                            //                 let mut mut_proof = app_proof.lock().await;
                            //                 *mut_proof = latest_proof;
                            //             }

                            //             // reset nonce
                            //             {
                            //                 let mut nonce = app_nonce.lock().await;
                            //                 *nonce = 0;
                            //             }
                            //             // reset epoch hashes
                            //             {
                            //                 info!("reset epoch hashes");
                            //                 let mut mut_epoch_hashes =
                            //                     app_epoch_hashes.write().await;
                            //                 mut_epoch_hashes.best_hash.solution = None;
                            //                 mut_epoch_hashes.best_hash.difficulty = 0;
                            //                 mut_epoch_hashes.submissions = HashMap::new();
                            //             }
                            //             break;
                            //         }
                            //     }
                            //     break;
                            // } else {
                            //     // sent error
                            //     if i >= 4 {
                            //         // warn!("Failed to send after 5 attempts. Discarding and refreshing data.");
                            //         // // MI: from time to time, rpc will rapidly fail 5 attempts, so the next part comment out
                            //         // // will end and fail the whole tx send-and-confirm in very short time.
                            //         // // reset nonce
                            //         // {
                            //         //     let mut nonce = app_nonce.lock().await;
                            //         //     *nonce = 0;
                            //         // }
                            //         // // reset epoch hashes
                            //         // {
                            //         //     info!("reset epoch hashes");
                            //         //     let mut mut_epoch_hashes = app_epoch_hashes.write().await;
                            //         //     mut_epoch_hashes.best_hash.solution = None;
                            //         //     mut_epoch_hashes.best_hash.difficulty = 0;
                            //         //     mut_epoch_hashes.submissions = HashMap::new();
                            //         // }

                            //         // // break for (0..5), re-enter loop to restart
                            //         // break;

                            //         // MI
                            //         // to repace above with next
                            //         warn!("Failed to send after 5 attempts. Re-entering loop and retrying with loading latest proof data.");
                            //         // break for (0..5), re-enter loop to restart
                            //         break;
                            //     }
                            // }

                            match rpc_client
                                .send_and_confirm_transaction_with_spinner_and_config(
                                    &tx,
                                    rpc_client.commitment(),
                                    send_cfg,
                                )
                                .await
                            {
                                Ok(sig) => {
                                    info!("Success!!");
                                    info!("Sig: {}", sig);
                                    if ! *no_sound_notification.clone() {
                                        utils::play_sound();
                                    }

                                    // update proof
                                    // limit number of checking no more than 10
                                    let mut num_checking = 0;
                                    loop {
                                        info!("Waiting for proof hash update");
                                        let latest_proof = { app_proof.lock().await.clone() };

                                        if old_proof.challenge.eq(&latest_proof.challenge) {
                                            info!("Proof challenge not updated yet..");
                                            old_proof = latest_proof;
                                            tokio::time::sleep(Duration::from_millis(1000)).await;
                                            num_checking += 1;
                                            if num_checking < 10 {
                                                continue;
                                            } else {
                                                info!("No proof hash update detected after 10 checkpoints. No more waiting, just keep going...");
                                                break;
                                            }
                                            // MI
                                            // continue;
                                        } else {
                                            info!(
                                                "Proof challenge updated! Checking rewards earned."
                                            );
                                            let balance = (latest_proof.balance as f64)
                                                / 10f64.powf(ORE_TOKEN_DECIMALS as f64);
                                            info!("New balance: {}", balance);
                                            let rewards = latest_proof.balance - old_proof.balance;
                                            let dec_rewards = (rewards as f64)
                                                / 10f64.powf(ORE_TOKEN_DECIMALS as f64);
                                            info!("Earned: {} ORE", dec_rewards);

                                            let submissions = {
                                                app_epoch_hashes.read().await.submissions.clone()
                                            };

                                            let mut total_hashpower: u64 = 0;

                                            for submission in submissions.iter() {
                                                total_hashpower += submission.1 .1
                                            }

                                            let _ = mine_success_sender.send(
                                                MessageInternalMineSuccess {
                                                    difficulty,
                                                    total_balance: balance,
                                                    rewards,
                                                    total_hashpower,
                                                    submissions,
                                                },
                                            );

                                            {
                                                let mut mut_proof = app_proof.lock().await;
                                                *mut_proof = latest_proof;
                                            }

                                            // reset nonce
                                            {
                                                let mut nonce = app_nonce.lock().await;
                                                *nonce = 0;
                                            }
                                            // reset epoch hashes
                                            {
                                                info!("reset epoch hashes");
                                                let mut mut_epoch_hashes =
                                                    app_epoch_hashes.write().await;
                                                mut_epoch_hashes.best_hash.solution = None;
                                                mut_epoch_hashes.best_hash.difficulty = 0;
                                                mut_epoch_hashes.submissions = HashMap::new();
                                            }
                                            break;
                                        }
                                    }
                                    break;
                                }

                                Err(err) => {
                                    match err.kind {
                                        ClientErrorKind::TransactionError(solana_sdk::transaction::TransactionError::InstructionError(_, err)) => {
                                            match err {
                                                // Custom instruction error, parse into OreError
                                                solana_program::instruction::InstructionError::Custom(err_code) => {
                                                    match err_code {
                                                        e if e == OreError::NeedsReset as u32 => {
                                                            error!("Ore: The epoch has ended and needs reset. Retrying...");
                                                            continue;
                                                        }
                                                        e if e == OreError::HashInvalid as u32 => {
                                                            error!("Ore: The provided hash is invalid. See you next solution.");
                                                            // reset nonce
                                                            {
                                                                let mut nonce = app_nonce.lock().await;
                                                                *nonce = 0;
                                                            }
                                                            // reset epoch hashes
                                                            {
                                                                info!("reset epoch hashes");
                                                                let mut mut_epoch_hashes =
                                                                    app_epoch_hashes.write().await;
                                                                mut_epoch_hashes.best_hash.solution = None;
                                                                mut_epoch_hashes.best_hash.difficulty = 0;
                                                                mut_epoch_hashes.submissions = HashMap::new();
                                                            }
                                                            // break for (0..5), re-enter outer loop to restart
                                                            break;
                                                        }
                                                        _ => {
                                                            error!("{}", &err.to_string());
                                                            continue;
                                                        }
                                                    }
                                                },

                                                // Non custom instruction error, return
                                                _ => {
                                                    error!("{}", &err.to_string());
                                                }
                                            }
                                        }

                                        // ClientErrorKind::Custom(ref err_code) => {
                                        //     match err_code {
                                        //         e if e.contains("custom program error: 0x0") => {
                                        //             error!("Ore: The epoch has ended and needs reset. Retrying...");
                                        //             continue;
                                        //         }
                                        //         e if e.contains("custom program error: 0x1") => {
                                        //             error!("Ore: The provided hash is invalid. See you next solution.");
                                        //             // reset nonce
                                        //             {
                                        //                 let mut nonce = app_nonce.lock().await;
                                        //                 *nonce = 0;
                                        //             }
                                        //             // reset epoch hashes
                                        //             {
                                        //                 info!("reset epoch hashes");
                                        //                 let mut mut_epoch_hashes =
                                        //                     app_epoch_hashes.write().await;
                                        //                 mut_epoch_hashes.best_hash.solution = None;
                                        //                 mut_epoch_hashes.best_hash.difficulty = 0;
                                        //                 mut_epoch_hashes.submissions = HashMap::new();
                                        //             }
                                        //             // break for (0..5), re-enter outer loop to restart
                                        //             break;
                                        //         }
                                        //         _ => {
                                        //             error!("{}", &err.to_string());
                                        //             continue;
                                        //         }
                                        //     }
                                        // }

                                        _ => {
                                            error!("{}", &err.to_string());
                                        }
                                    }

                                    if i >= 4 {
                                        warn!("Failed to send after 5 attempts. Discarding and refreshing data.");
                                        // MI: from time to time, rpc will rapidly fail 5 attempts, so the next part comment out
                                        // will end and fail the whole tx send-and-confirm in very short time.
                                        // reset nonce
                                        {
                                            let mut nonce = app_nonce.lock().await;
                                            *nonce = 0;
                                        }
                                        // reset epoch hashes
                                        {
                                            info!("reset epoch hashes");
                                            let mut mut_epoch_hashes = app_epoch_hashes.write().await;
                                            mut_epoch_hashes.best_hash.solution = None;
                                            mut_epoch_hashes.best_hash.difficulty = 0;
                                            mut_epoch_hashes.submissions = HashMap::new();
                                        }

                                        // break for (0..5), re-enter outer loop to restart
                                        break;

                                        // // MI
                                        // // to repace above with next
                                        // warn!("Failed to send after 5 attempts. Re-entering loop and retrying with loading latest proof data.");
                                        // // break for (0..5), re-enter loop to restart
                                        // break;
                                    }
                                }
                            }
                            tokio::time::sleep(Duration::from_millis(1000)).await;
                        } else {
                            error!("Failed to get latest blockhash. retrying...");
                            tokio::time::sleep(Duration::from_millis(1000)).await;
                        }
                    }
                } else {
                    // error!("No best solution received yet.");
                    tokio::time::sleep(Duration::from_millis(1000)).await;
                }
            } else {
                tokio::time::sleep(Duration::from_secs(cutoff as u64)).await;
            };
        }
    });

    let app_shared_state = shared_state.clone();
    let app_rpc_client = rpc_client.clone();
    let app_wallet = wallet_extension.clone();
    tokio::spawn(async move {
        loop {
            while let Some(msg) = mine_success_receiver.recv().await {
                {
                    let decimals = 10f64.powf(ORE_TOKEN_DECIMALS as f64);
                    let pool_rewards_dec = (msg.rewards as f64).div(decimals);
                    let shared_state = app_shared_state.read().await;
                    for (_socket_addr, socket_sender) in shared_state.sockets.iter() {
                        let pubkey = socket_sender.0;

                        if let Some((supplied_diff, _pubkey_hashpower)) =
                            msg.submissions.get(&pubkey)
                        {
                            // let hashpower_percent =
                            //     (*pubkey_hashpower as f64).div(msg.total_hashpower as f64);

                            let hashpower =
                                MIN_HASHPOWER * 2u64.pow(*supplied_diff as u32 - MIN_DIFF);

                            let hashpower_percent = (hashpower as u128)
                                .saturating_mul(1_000_000)
                                .saturating_div(msg.total_hashpower as u128);

                            // let decimals = 10f64.powf(ORE_TOKEN_DECIMALS as f64);
                            // let earned_rewards = hashpower_percent
                            //     .mul(msg.rewards)
                            //     .mul(decimals)
                            //     .floor()
                            //     .div(decimals);
                            let earned_rewards = hashpower_percent
                                .saturating_mul(msg.rewards as u128)
                                .saturating_div(1_000_000)
                                as u64;

                            if let Some(socket_sender) = shared_state.miner_sockets.get(&pubkey) {
                                let earned_rewards_dec = (earned_rewards as f64).div(decimals);

                                let message = format!(
                                    "Submitted Difficulty: {}\nPool Earned: {} ORE.\nPool Balance: {}\nMiner Earned: {} ORE for difficulty: {}",
                                    msg.difficulty,
                                    pool_rewards_dec,
                                    msg.total_balance,
                                    earned_rewards_dec,
                                    supplied_diff
                                );
                                if let Ok(_) = socket_sender
                                    .lock()
                                    .await
                                    .send(Message::Text(message))
                                    .await
                                {
                                } else {
                                    println!("Failed to send client text");
                                }
                            }
                        }
                    }
                }
                if let Ok(balance) = app_rpc_client.get_balance(&app_wallet.pubkey()).await {
                    info!(
                        "Sol Balance: {:.2}",
                        balance as f64 / LAMPORTS_PER_SOL as f64
                    );
                } else {
                    error!("Failed to load balance");
                };
            }
        }
    });

    let client_channel = client_message_sender.clone();
    let app_shared_state = shared_state.clone();
    let app = Router::new()
        .route("/", get(ws_handler))
        .route("/latest-blockhash", get(get_latest_blockhash))
        .route("/pool/authority/pubkey", get(get_pool_authority_pubkey))
        .with_state(app_shared_state)
        .layer(Extension(wallet_extension))
        .layer(Extension(client_channel))
        .layer(Extension(rpc_client))
        .layer(Extension(client_nonce_ranges))
        // Logging
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::default().include_headers(true)),
        );

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();

    tracing::debug!("listening on {}", listener.local_addr().unwrap());

    let app_shared_state = shared_state.clone();
    tokio::spawn(async move {
        ping_check_system(&app_shared_state).await;
    });

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();

    Ok(())
}

async fn get_pool_authority_pubkey(
    Extension(wallet): Extension<Arc<Keypair>>,
) -> impl IntoResponse {
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/text")
        .body(wallet.pubkey().to_string())
        .unwrap()
}

async fn get_latest_blockhash(
    Extension(rpc_client): Extension<Arc<RpcClient>>,
) -> impl IntoResponse {
    let latest_blockhash = rpc_client.get_latest_blockhash().await.unwrap();

    let serialized_blockhash = bincode::serialize(&latest_blockhash).unwrap();

    let encoded_blockhash = BASE64_STANDARD.encode(serialized_blockhash);
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/text")
        .body(encoded_blockhash)
        .unwrap()
}

#[derive(Deserialize)]
struct WsQueryParams {
    timestamp: u64,
}

#[debug_handler]
async fn ws_handler(
    ws: WebSocketUpgrade,
    TypedHeader(auth_header): TypedHeader<axum_extra::headers::Authorization<Basic>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(app_state): State<Arc<RwLock<AppState>>>,
    Extension(client_channel): Extension<UnboundedSender<ClientMessage>>,
    query_params: Query<WsQueryParams>,
) -> impl IntoResponse {
    let msg_timestamp = query_params.timestamp;

    let pubkey = auth_header.username();
    let signed_msg = auth_header.password();

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_secs();

    // Signed authentication message is only valid for 5 seconds
    if (now - query_params.timestamp) >= 5 {
        return Err((StatusCode::UNAUTHORIZED, "Timestamp too old."));
    }

    // verify client
    if let Ok(pubkey) = Pubkey::from_str(pubkey) {
        if let Ok(signature) = Signature::from_str(signed_msg) {
            let ts_msg = msg_timestamp.to_le_bytes();

            if signature.verify(&pubkey.to_bytes(), &ts_msg) {
                println!("Client: {addr} connected with pubkey {pubkey}.");
                return Ok(ws.on_upgrade(move |socket| {
                    handle_socket(socket, addr, pubkey, app_state, client_channel)
                }));
            } else {
                return Err((StatusCode::UNAUTHORIZED, "Sig verification failed"));
            }
        } else {
            return Err((StatusCode::UNAUTHORIZED, "Invalid signature"));
        }
    } else {
        return Err((StatusCode::UNAUTHORIZED, "Invalid pubkey"));
    }
}

async fn handle_socket(
    mut socket: WebSocket,
    who: SocketAddr,
    who_pubkey: Pubkey,
    rw_app_state: Arc<RwLock<AppState>>,
    client_channel: UnboundedSender<ClientMessage>,
) {
    if socket
        .send(axum::extract::ws::Message::Ping(vec![1, 2, 3]))
        .await
        .is_ok()
    {
        println!("Pinged {who}... pubkey: {who_pubkey}");
    } else {
        println!("could not ping {who} pubkey: {who_pubkey}");

        // if we can't ping we can't do anything, return to close the connection
        return;
    }

    let (sender, mut receiver) = socket.split();
    let mut app_state = rw_app_state.write().await;
    if app_state.sockets.contains_key(&who) {
        println!("Socket addr: {who} already has an active connection");
        return;
    } else {
        let sender = Arc::new(Mutex::new(sender));
        app_state.sockets.insert(who, (who_pubkey, sender.clone()));
        app_state.miner_sockets.insert(who_pubkey, sender);
    }
    drop(app_state);

    let _ = tokio::spawn(async move {
        // MI: vanilla. by design while let will exit when None received
        while let Some(Ok(msg)) = receiver.next().await {
            if process_message(msg, who, client_channel.clone()).is_break() {
                break;
            }
        }

        // // MI: use loop, since by design while let will exit when None received
        // loop {
        //     if let Some(Ok(msg)) = receiver.next().await {
        //         if process_message(msg, who, client_channel.clone()).is_break() {
        //             break;
        //         }
        //     }
        // }
    })
    .await;

    let mut app_state = rw_app_state.write().await;
    app_state.sockets.remove(&who);
    app_state.miner_sockets.remove(&who_pubkey);
    drop(app_state);

    info!("Client: {} disconnected!", who_pubkey.to_string());
}

fn process_message(
    msg: Message,
    who: SocketAddr,
    client_channel: UnboundedSender<ClientMessage>,
) -> ControlFlow<(), ()> {
    match msg {
        Message::Text(t) => {
            println!(">>> {who} sent str: {t:?}");
        }
        Message::Binary(d) => {
            // first 8 bytes are message type
            let message_type = d[0];
            match message_type {
                0 => {
                    // println!("Got Ready message");
                    let mut b_index = 1;

                    let mut pubkey = [0u8; 32];
                    for i in 0..32 {
                        pubkey[i] = d[i + b_index];
                    }
                    b_index += 32;

                    let mut ts = [0u8; 8];
                    for i in 0..8 {
                        ts[i] = d[i + b_index];
                    }

                    let ts = u64::from_le_bytes(ts);

                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .expect("Time went backwards")
                        .as_secs();

                    let time_since = now - ts;
                    if time_since > 5 {
                        error!("Client tried to ready up with expired signed message");
                        return ControlFlow::Break(());
                    }

                    let msg = ClientMessage::Ready(who);
                    let _ = client_channel.send(msg);
                }
                1 => {
                    let msg = ClientMessage::Mining(who);
                    let _ = client_channel.send(msg);
                }
                2 => {
                    // parse solution from message data
                    let mut solution_bytes = [0u8; 16];
                    // extract (16 u8's) from data for hash digest
                    let mut b_index = 1;
                    for i in 0..16 {
                        solution_bytes[i] = d[i + b_index];
                    }
                    b_index += 16;

                    // extract 64 bytes (8 u8's)
                    let mut nonce = [0u8; 8];
                    for i in 0..8 {
                        nonce[i] = d[i + b_index];
                    }
                    b_index += 8;

                    let mut pubkey = [0u8; 32];
                    for i in 0..32 {
                        pubkey[i] = d[i + b_index];
                    }

                    b_index += 32;

                    let signature_bytes = d[b_index..].to_vec();
                    if let Ok(sig_str) = String::from_utf8(signature_bytes.clone()) {
                        if let Ok(sig) = Signature::from_str(&sig_str) {
                            let pubkey = Pubkey::new_from_array(pubkey);

                            let mut hash_nonce_message = [0; 24];
                            hash_nonce_message[0..16].copy_from_slice(&solution_bytes);
                            hash_nonce_message[16..24].copy_from_slice(&nonce);

                            if sig.verify(&pubkey.to_bytes(), &hash_nonce_message) {
                                let solution = Solution::new(solution_bytes, nonce);

                                let msg = ClientMessage::BestSolution(who, solution, pubkey);
                                let _ = client_channel.send(msg);
                            } else {
                                error!("Client submission sig verification failed.");
                            }
                        } else {
                            error!("Failed to parse into Signature.");
                        }
                    } else {
                        error!("Failed to parse signed message from client.");
                    }
                }
                _ => {
                    println!(">>> {} sent an invalid message", who);
                }
            }
        }
        Message::Close(c) => {
            if let Some(cf) = c {
                println!(
                    ">>> {} sent close with code {} and reason `{}`",
                    who, cf.code, cf.reason
                );
            } else {
                println!(">>> {who} somehow sent close message without CloseFrame");
            }
            return ControlFlow::Break(());
        }
        Message::Pong(_v) => {
            //println!(">>> {who} sent pong with {v:?}");
        }
        Message::Ping(_v) => {
            //println!(">>> {who} sent ping with {v:?}");
        }
    }

    ControlFlow::Continue(())
}

async fn proof_tracking_system(ws_url: String, wallet: Arc<Keypair>, proof: Arc<Mutex<Proof>>) {
    loop {
        println!("Establishing rpc websocket connection...");
        let mut ps_client = PubsubClient::new(&ws_url).await;
        // let mut attempts = 0;

        // while ps_client.is_err() && attempts < 3 {
        while ps_client.is_err() {
            error!("Failed to connect to websocket, retrying...");
            ps_client = PubsubClient::new(&ws_url).await;
            tokio::time::sleep(Duration::from_millis(2000)).await;
            // attempts += 1;
        }
        info!("RPC WS connection established!");

        let app_wallet = wallet.clone();
        if let Ok(ps_client) = ps_client {
            // The `PubsubClient` must be `Arc`ed to share it across threads/tasks.
            let ps_client = Arc::new(ps_client);
            let app_proof = proof.clone();
            let account_pubkey = proof_pubkey(app_wallet.pubkey());
            let pubsub = ps_client
                .account_subscribe(
                    &account_pubkey,
                    Some(RpcAccountInfoConfig {
                        encoding: Some(UiAccountEncoding::Base64),
                        data_slice: None,
                        commitment: Some(CommitmentConfig::confirmed()),
                        min_context_slot: None,
                    }),
                )
                .await;

            info!("Subscribed tracking pool proof updates with websocket");
            if let Ok((mut account_sub_notifications, _account_unsub)) = pubsub {
                while let Some(response) = account_sub_notifications.next().await {
                    let data = response.value.data.decode();
                    if let Some(data_bytes) = data {
                        if let Ok(new_proof) = Proof::try_from_bytes(&data_bytes) {
                            {
                                let mut app_proof = app_proof.lock().await;
                                *app_proof = *new_proof;
                                drop(app_proof);
                            }
                        }
                    }
                }
            }
        }
    }
}

async fn client_message_handler_system(
    mut receiver_channel: UnboundedReceiver<ClientMessage>,
    shared_state: &Arc<RwLock<AppState>>,
    epoch_hashes: Arc<RwLock<EpochHashes>>,
    ready_clients: Arc<Mutex<HashSet<SocketAddr>>>,
    proof: Arc<Mutex<Proof>>,
    client_nonce_ranges: Arc<RwLock<HashMap<Pubkey, Range<u64>>>>,
    min_difficulty: u32,
) {
    while let Some(client_message) = receiver_channel.recv().await {
        match client_message {
            ClientMessage::Ready(addr) => {
                info!("Client {} is ready!", addr.to_string());
                {
                    let shared_state = shared_state.read().await;
                    if let Some(sender) = shared_state.sockets.get(&addr) {
                        {
                            let mut ready_clients = ready_clients.lock().await;
                            ready_clients.insert(addr);
                        }

                        if let Ok(_) = sender
                            .1
                            .lock()
                            .await
                            .send(Message::Text(String::from("Client successfully added.")))
                            .await
                        {
                        } else {
                            println!("Failed notify client they were readied up!");
                        }
                    }
                }
            }
            ClientMessage::Mining(addr) => {
                println!("Client {} has started mining!", addr.to_string());
            }
            ClientMessage::BestSolution(_addr, solution, pubkey) => {
                let pubkey_str = pubkey.to_string();
                let challenge = {
                    let proof = proof.lock().await;
                    proof.challenge
                };

                let nonce_range: Range<u64> = {
                    if let Some(nr) = client_nonce_ranges.read().await.get(&pubkey) {
                        nr.clone()
                    } else {
                        error!("Client nonce range not set!");
                        continue;
                    }
                };

                let nonce = u64::from_le_bytes(solution.n);

                if !nonce_range.contains(&nonce) {
                    error!("Client submitted nonce out of assigned range");
                    continue;
                }

                if solution.is_valid(&challenge) {
                    let diff = solution.to_hash().difficulty();
                    info!("{} found diff: {}", pubkey_str, diff);
                    // if diff >= MIN_DIFF {
                    if diff >= min_difficulty {
                        // calculate rewards
                        // let hashpower = MIN_HASHPOWER * 2u64.pow(diff - min_difficulty);
                        let hashpower = MIN_HASHPOWER * 2u64.pow(diff - MIN_DIFF);

                        {
                            let mut epoch_hashes = epoch_hashes.write().await;
                            epoch_hashes.submissions.insert(pubkey, (diff, hashpower));
                            if diff > epoch_hashes.best_hash.difficulty {
                                epoch_hashes.best_hash.difficulty = diff;
                                epoch_hashes.best_hash.solution = Some(solution);
                            }
                        }
                    } else {
                        error!("Diff too low, skipping");
                    }
                } else {
                    error!(
                        "{} returned a solution which is invalid for the latest challenge!",
                        pubkey
                    );
                    // MI: return will stop this spawned thread and stop mining process
                    // return;
                }
            }
        }
    }
}

async fn ping_check_system(shared_state: &Arc<RwLock<AppState>>) {
    loop {
        // send ping to all sockets
        let mut failed_sockets = Vec::new();
        let app_state = shared_state.read().await;
        // I don't like doing all this work while holding this lock...
        for (who, socket) in app_state.sockets.iter() {
            if socket
                .1
                .lock()
                .await
                .send(Message::Ping(vec![1, 2, 3]))
                .await
                .is_ok()
            {
                //println!("Pinged: {who}...");
            } else {
                failed_sockets.push(who.clone());
            }
        }
        drop(app_state);

        // remove any sockets where ping failed
        let mut app_state = shared_state.write().await;
        for address in failed_sockets {
            app_state.sockets.remove(&address);
        }
        drop(app_state);

        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

fn styles() -> Styles {
    Styles::styled()
        .header(AnsiColor::Red.on_default() | Effects::BOLD)
        .usage(AnsiColor::Red.on_default() | Effects::BOLD)
        .literal(AnsiColor::Blue.on_default() | Effects::BOLD)
        .placeholder(AnsiColor::Green.on_default())
}
