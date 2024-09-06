/*
    db initialization
     sudo -i -u postgres
     psql
     or
    sudo -i -u postgres psql

    CREATE ROLE miracle LOGIN CREATEDB CREATEROLE;
    ALTER USER miracle WITH PASSWORD 'Mirascape';
    psql postgres -U miracle;
    OR
    psql -U miracle -h localhost -d postgres

    CREATE DATABASE miraland;
    or
    createdb miraland --owner miracle

    up.sql
    GRANT ALL ON SCHEMA ore TO <other_user>;

    DATABASE_URL=postgres://username:password@localhost/mydb
*/

CREATE DATABASE miraland;
DROP SCHEMA IF EXISTS ore;
CREATE SCHEMA IF NOT EXISTS ore AUTHORIZATION miracle;

DROP TABLE IF EXISTS ore.miners;
DROP TABLE IF EXISTS ore.pools;
DROP TABLE IF EXISTS ore.challenges;
DROP TABLE IF EXISTS ore.submissions;
DROP TABLE IF EXISTS ore.transactions;
DROP TABLE IF EXISTS ore.claims;
DROP TABLE IF EXISTS ore.rewards;
DROP TABLE IF EXISTS ore.earnings;


/*
    id - primary key, auto created index
    PostgreSQL automatically creates a unique index when a unique constraint or primary key is defined for a table.
    user index naming convention(with prefix):
    pkey_: primary key
    ukey_: user key
    fkey_: foreign key
    uniq_: unique index
    indx_: other index (multiple values)

    status: Enrolled, Activated, Frozen, Deactivated
*/

CREATE TABLE ore.miners (
    id INT PRIMARY KEY GENERATED ALWAYS AS IDENTITY
        (START WITH 1000 INCREMENT BY 1),
    pubkey VARCHAR(44) NOT NULL,
    enabled BOOL DEFAULT false NOT NULL,
    status: VARCHAR(30) DEFAULT 'Enrolled' NOT NULL,
    created TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL,
    updated TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP NOT NULL
);

CREATE UNIQUE INDEX uniq_miners_pubkey ON ore.miners (pubkey ASC);


CREATE TABLE ore.pools (
    id INT PRIMARY KEY GENERATED ALWAYS AS IDENTITY
        (START WITH 1000 INCREMENT BY 1),
    proof_pubkey VARCHAR(44) NOT NULL,
    authority_pubkey VARCHAR(44) NOT NULL,
    total_rewards BIGINT DEFAULT 0 NOT NULL,
    claimed_rewards BIGINT DEFAULT 0 NOT NULL,
    created TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL,
    updated TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP NOT NULL
);

CREATE INDEX indx_pools_proof_pubkey ON ore.pools (proof_pubkey ASC);
CREATE INDEX indx_pools_authority_pubkey ON ore.pools (authority_pubkey ASC);


CREATE TABLE ore.challenges (
    id INT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
    pool_id INT NOT NULL,
    submission_id INT,
    challenge BYTEA NOT NULL,
    rewards_earned BIGINT UNSIGNED DEFAULT 0,
    created TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL,
    updated TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP NOT NULL
);

CREATE INDEX indx_challenges_challenge ON ore.challenges (challenge ASC);


CREATE TABLE ore.submissions (
  id INT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
  miner_id INT NOT NULL,
  challenge_id INT NOT NULL,
  difficulty SMALLINT NOT NULL,
  nonce BIGINT NOT NULL,
  digest BYTEA NOT NULL,
  created TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL,
  updated TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP NOT NULL
);

CREATE INDEX indx_submissions_miner_challenge_ids ON ore.submissions (miner_id ASC, challenge_id ASC)
CREATE INDEX indx_submissions_nonce ON ore.submissions (nonce ASC);


CREATE TABLE ore.transactions (
  id INT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
  transaction_type VARCHAR(30) NOT NULL,
  signature VARCHAR(200) NOT NULL,
  priority_fee INT DEFAULT 0 NOT NULL,
  pool_id INT NOT NULL,
  miner_id INT,
  created TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL,
  updated TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP NOT NULL
);

CREATE INDEX indx_transactions_signature ON ore.transactions (signature ASC);
CREATE INDEX indx_transactions_pool_id_created ON ore.transactions (pool_id ASC, transaction_type ASC, created DESC);
CREATE INDEX indx_transactions_miner_id_created ON ore.transactions (miner_id ASC, created DESC);


CREATE TABLE ore.claims (
  id INT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
  miner_id INT NOT NULL,
  pool_id INT NOT NULL,
  transaction_id INT NOT NULL,
  amount BIGINT NOT NULL,
  created TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL,
  updated TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP NOT NULL
);

CREATE INDEX indx_claims_miner_pool_txn_ids ON ore.claims (miner_id ASC, pool_id ASC, transaction_id ASC);


CREATE TABLE ore.rewards (
  id INT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
  miner_id INT NOT NULL,
  pool_id INT NOT NULL,
  balance BIGINT DEFAULT 0 NOT NULL,
  created TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL,
  updated TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP NOT NULL
);

CREATE INDEX indx_rewards_miner_pool_ids ON ore.rewards (miner_id ASC, pool_id ASC);


CREATE TABLE ore.earnings (
  id INT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
  miner_id INT NOT NULL,
  pool_id INT NOT NULL,
  challenge_id INT NOT NULL,
  amount BIGINT DEFAULT 0 NOT NULL,
  created TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL,
  updated TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP NOT NULL
);

CREATE INDEX indx_earnings_miner_pool_challenge_ids ON ore.earnings (miner_id ASC, pool_id ASC, challenge_id ASC);
