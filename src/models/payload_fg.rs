use sha2::{Digest, Sha256};

pub fn compute_transfer_hash(
	source_id: i32,
	dest_id: i32,
	amount: rust_decimal::Decimal,
	description: &str,
) -> Vec<u8> {
	let mut hasher = Sha256::new();
	hasher.update(source_id.to_le_bytes());
	hasher.update(dest_id.to_le_bytes());
	hasher.update(amount.to_string().as_bytes());
	hasher.update(description.trim().as_bytes());
	hasher.finalize().to_vec()
}

pub fn compute_reversal_hash(target_txn_id: i32, reason: &str) -> Vec<u8> {
	let mut hasher = Sha256::new();
	hasher.update(target_txn_id.to_le_bytes());
	hasher.update(reason.trim().as_bytes());
	hasher.finalize().to_vec()
}