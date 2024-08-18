use cached::proc_macro::cached;
use std::{
    io::Cursor,
    time::{SystemTime, UNIX_EPOCH},
};

use drillx::Solution;
use ore_api::{
    consts::{
        BUS_ADDRESSES, BUS_COUNT, CONFIG_ADDRESS, MINT_ADDRESS, PROOF, TOKEN_DECIMALS,
        TREASURY_ADDRESS,
    },
    instruction,
    state::{Bus, Config, Proof},
};
pub use ore_utils::AccountDeserialize;
use rand::Rng;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{instruction::Instruction, pubkey::Pubkey};
use spl_associated_token_account::get_associated_token_address;

pub const ORE_TOKEN_DECIMALS: u8 = TOKEN_DECIMALS;

pub fn get_auth_ix(signer: Pubkey) -> Instruction {
    let proof = proof_pubkey(signer);

    instruction::auth(proof)
}

pub fn get_mine_ix(signer: Pubkey, solution: Solution, bus: usize) -> Instruction {
    instruction::mine(signer, signer, BUS_ADDRESSES[bus], solution)
}

pub fn get_register_ix(signer: Pubkey) -> Instruction {
    instruction::open(signer, signer, signer)
}

pub async fn _get_config(client: &RpcClient) -> Result<ore_api::state::Config, String> {
    let data = client.get_account_data(&CONFIG_ADDRESS).await;
    match data {
        Ok(data) => {
            let config = Config::try_from_bytes(&data);
            if let Ok(config) = config {
                return Ok(*config);
            } else {
                return Err("Failed to parse config account".to_string());
            }
        }
        Err(_) => return Err("Failed to get config account".to_string()),
    }
}

// MI
pub async fn get_proof_and_best_bus(
    client: &RpcClient,
    authority: Pubkey,
) -> Result<(Proof, (/* bus index: */ usize, /* bus address: */ Pubkey)), ()> {
    let account_pubkeys = vec![
        proof_pubkey(authority),
        BUS_ADDRESSES[0],
        BUS_ADDRESSES[1],
        BUS_ADDRESSES[2],
        BUS_ADDRESSES[3],
        BUS_ADDRESSES[4],
        BUS_ADDRESSES[5],
        BUS_ADDRESSES[6],
        BUS_ADDRESSES[7],
    ];
    let accounts = client.get_multiple_accounts(&account_pubkeys).await;
    if let Ok(accounts) = accounts {
        let proof = if let Some(account) = &accounts[0] {
            *Proof::try_from_bytes(&account.data).expect("Failed to parse proof account")
        } else {
            return Err(());
        };

        // Fetch the bus with the largest balance
        let mut top_bus_balance: u64 = 0;
        let mut top_bus_id = 0;
        let mut top_bus = BUS_ADDRESSES[0];
        for account in &accounts[1..] {
            if let Some(account) = account {
                if let Ok(bus) = Bus::try_from_bytes(&account.data) {
                    if bus.rewards.gt(&top_bus_balance) {
                        top_bus_balance = bus.rewards;
                        top_bus_id = bus.id as usize;
                        top_bus = BUS_ADDRESSES[top_bus_id];
                    }
                } else {
                    return Err(());
                }
            }
        }

        Ok((proof, (top_bus_id, top_bus)))
    } else {
        Err(())
    }
}

// MI
async fn _find_bus(rpc_client: &RpcClient) -> Pubkey {
    // Fetch the bus with the largest balance
    if let Ok(accounts) = rpc_client.get_multiple_accounts(&BUS_ADDRESSES).await {
        let mut top_bus_balance: u64 = 0;
        let mut top_bus = BUS_ADDRESSES[0];
        for account in accounts {
            if let Some(account) = account {
                if let Ok(bus) = Bus::try_from_bytes(&account.data) {
                    if bus.rewards.gt(&top_bus_balance) {
                        top_bus_balance = bus.rewards;
                        top_bus = BUS_ADDRESSES[bus.id as usize];
                    }
                }
            }
        }
        return top_bus;
    }

    // Otherwise return a random bus
    let i = rand::thread_rng().gen_range(0..BUS_COUNT);
    BUS_ADDRESSES[i]
}

pub async fn get_proof(client: &RpcClient, authority: Pubkey) -> Result<Proof, String> {
    let proof_address = proof_pubkey(authority);
    let data = client.get_account_data(&proof_address).await;
    match data {
        Ok(data) => {
            let proof = Proof::try_from_bytes(&data);
            if let Ok(proof) = proof {
                return Ok(*proof);
            } else {
                return Err("Failed to parse proof account".to_string());
            }
        }
        Err(_) => return Err("Failed to get proof account".to_string()),
    }
}

pub fn get_cutoff(proof: Proof, buffer_time: u64) -> i64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Failed to get time")
        .as_secs() as i64;
    proof
        .last_hash_at
        .saturating_add(60)
        .saturating_sub(buffer_time as i64)
        .saturating_sub(now)
}

// MI
// #[cached]
pub fn play_sound() {
    match rodio::OutputStream::try_default() {
        Ok((_stream, handle)) => {
            let sink = rodio::Sink::try_new(&handle).unwrap();
            let bytes = include_bytes!("../assets/success.mp3");
            let cursor = Cursor::new(bytes);
            sink.append(rodio::Decoder::new(cursor).unwrap());
            sink.sleep_until_end();
        }
        Err(_) => print!("\x07"),
    }
}

#[cached]
pub fn proof_pubkey(authority: Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[PROOF, authority.as_ref()], &ore_api::ID).0
}

#[cached]
pub fn treasury_tokens_pubkey() -> Pubkey {
    get_associated_token_address(&TREASURY_ADDRESS, &MINT_ADDRESS)
}
