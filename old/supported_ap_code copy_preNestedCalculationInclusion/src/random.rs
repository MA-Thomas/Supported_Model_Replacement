use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

pub(crate) fn rng_for(seed: u64, domain: u64, indices: &[usize]) -> ChaCha8Rng {
    let mut mixed = splitmix64(seed ^ domain);
    for &index in indices {
        mixed = splitmix64(mixed ^ index as u64);
    }
    ChaCha8Rng::seed_from_u64(mixed)
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = value;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}
