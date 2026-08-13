use crate::{Parallelism, SupportedApError};

pub(crate) fn map_init_indexed<R, State, Init, Op>(
    len: usize,
    parallelism: Parallelism,
    init: Init,
    op: Op,
) -> Result<Vec<R>, SupportedApError>
where
    R: Send,
    State: Send,
    Init: Fn() -> State + Sync + Send,
    Op: Fn(&mut State, usize) -> R + Sync + Send,
{
    match parallelism {
        Parallelism::Sequential => Ok(sequential(len, init, op)),
        Parallelism::Auto => auto(len, init, op),
        Parallelism::Threads(threads) => with_threads(len, threads.get(), init, op),
    }
}

fn sequential<R, State, Init, Op>(len: usize, init: Init, op: Op) -> Vec<R>
where
    Init: Fn() -> State,
    Op: Fn(&mut State, usize) -> R,
{
    let mut state = init();
    (0..len).map(|index| op(&mut state, index)).collect()
}

#[cfg(feature = "parallel")]
fn auto<R, State, Init, Op>(len: usize, init: Init, op: Op) -> Result<Vec<R>, SupportedApError>
where
    R: Send,
    State: Send,
    Init: Fn() -> State + Sync + Send,
    Op: Fn(&mut State, usize) -> R + Sync + Send,
{
    use rayon::prelude::*;

    Ok((0..len)
        .into_par_iter()
        .map_init(init, op)
        .collect::<Vec<_>>())
}

#[cfg(not(feature = "parallel"))]
fn auto<R, State, Init, Op>(len: usize, init: Init, op: Op) -> Result<Vec<R>, SupportedApError>
where
    R: Send,
    State: Send,
    Init: Fn() -> State + Sync + Send,
    Op: Fn(&mut State, usize) -> R + Sync + Send,
{
    Ok(sequential(len, init, op))
}

#[cfg(feature = "parallel")]
fn with_threads<R, State, Init, Op>(
    len: usize,
    threads: usize,
    init: Init,
    op: Op,
) -> Result<Vec<R>, SupportedApError>
where
    R: Send,
    State: Send,
    Init: Fn() -> State + Sync + Send,
    Op: Fn(&mut State, usize) -> R + Sync + Send,
{
    use rayon::prelude::*;

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .map_err(|error| SupportedApError::ThreadPoolBuild {
            message: error.to_string(),
        })?;
    Ok(pool.install(|| {
        (0..len)
            .into_par_iter()
            .map_init(init, op)
            .collect::<Vec<_>>()
    }))
}

#[cfg(not(feature = "parallel"))]
fn with_threads<R, State, Init, Op>(
    _len: usize,
    _threads: usize,
    _init: Init,
    _op: Op,
) -> Result<Vec<R>, SupportedApError>
where
    R: Send,
    State: Send,
    Init: Fn() -> State + Sync + Send,
    Op: Fn(&mut State, usize) -> R + Sync + Send,
{
    Err(SupportedApError::ParallelFeatureDisabled)
}
