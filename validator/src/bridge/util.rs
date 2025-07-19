use solana_sdk::signature::{Keypair, SeedDerivable};

pub fn keypair_from_seed(seed: &[u8; 32]) -> Keypair {
    Keypair::from_seed(seed).unwrap()
}

pub fn mint_keypair() -> Keypair {
    let seed_phrase = "THERAINISME.MINT";
    let mut seed = [0u8; 32];
    let phrase_bytes = seed_phrase.as_bytes();
    let len = std::cmp::min(phrase_bytes.len(), 32);
    seed[..len].copy_from_slice(&phrase_bytes[..len]);
    keypair_from_seed(&seed)
}

pub fn faucet_keypair() -> Keypair {
    let seed_phrase = "THERAINISME.FAUCET";
    let mut seed = [0u8; 32];
    let phrase_bytes = seed_phrase.as_bytes();
    let len = std::cmp::min(phrase_bytes.len(), 32);
    seed[..len].copy_from_slice(&phrase_bytes[..len]);
    keypair_from_seed(&seed)
}
