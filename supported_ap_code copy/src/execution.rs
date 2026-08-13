use crate::Execution;

pub(crate) fn map_indices<T, F>(count: usize, execution: Execution, function: F) -> Vec<T>
where
    T: Send,
    F: Fn(usize) -> T + Sync + Send,
{
    #[cfg(feature = "parallel")]
    {
        if execution == Execution::Parallel {
            use rayon::prelude::*;
            return (0..count).into_par_iter().map(function).collect();
        }
    }
    let _ = execution;
    (0..count).map(function).collect()
}
