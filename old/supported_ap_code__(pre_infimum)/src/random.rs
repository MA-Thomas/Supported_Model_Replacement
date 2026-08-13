use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

pub(crate) const BOOTSTRAP_DOMAIN: u64 = 0x4253_5452_4150_3031;
pub(crate) const PERMUTATION_LABEL_DOMAIN: u64 = 0x5045_524d_4c41_424c;
pub(crate) const PERMUTATION_BOOTSTRAP_DOMAIN: u64 = 0x5045_524d_4253_5452;
pub(crate) const PAIRED_BOOTSTRAP_DOMAIN: u64 = 0x5041_4952_4253_5452;

#[inline]
pub(crate) fn rng_for(seed: u64, domain: u64, outer: usize, inner: usize) -> ChaCha8Rng {
    ChaCha8Rng::seed_from_u64(derived_seed(seed, domain, outer, inner))
}

#[inline]
fn derived_seed(seed: u64, domain: u64, outer: usize, inner: usize) -> u64 {
    let mixed = splitmix64(seed ^ domain);
    let mixed = splitmix64(mixed ^ outer as u64);
    splitmix64(mixed ^ inner as u64)
}

#[inline]
fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}
