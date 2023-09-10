use std::{iter, num::TryFromIntError};

pub fn rel_dt_to_abs(dt: &[i16], avg: u32) -> Vec<u64> {
    dt.iter()
        .enumerate()
        .map(|(i, dt)| {
            (i as u64 * avg as u64)
                .checked_add_signed(*dt as i64)
                .unwrap()
        })
        .collect()
}

pub fn calc_avg_dt(abs: &[u64]) -> Option<u32> {
    if abs.len() == 0 {
        return Some(0);
    }
    u32::try_from(
        iter::once(0u64)
            .chain(abs.iter().copied())
            .zip(abs.iter().copied())
            .map(|(last, next)| (next - last) as u128)
            .sum::<u128>()
            / abs.len() as u128,
    )
    .ok()
}

pub fn calc_rel_dt(avg: u32, abs: &[u64]) -> Option<Vec<i16>> {
    let rel = abs
        .iter()
        .enumerate()
        .map(|(i, abs_dt)| i16::try_from(*abs_dt as i64 - (avg as u64 * i as u64) as i64))
        .collect::<Result<Vec<i16>, TryFromIntError>>()
        .ok()?;
    debug_assert_eq!(abs, rel_dt_to_abs(&rel, avg));
    Some(rel)
}
