//! Row-parallel helpers. With the `parallel` feature rows are spread over
//! rayon's pool; without it (or on single-threaded wasm) they run in order.
//! Kernels must be order-independent per row so results never depend on it.

/// Calls `f(y, row)` for every `width`-sized row of `data`.
#[inline]
pub(crate) fn for_rows<T: Send, F>(data: &mut [T], width: usize, f: F)
where
    F: Fn(usize, &mut [T]) + Sync + Send,
{
    if width == 0 {
        return;
    }
    #[cfg(feature = "parallel")]
    {
        use rayon::prelude::*;
        // Small images aren't worth the scheduling cost.
        if data.len() >= 64 * 1024 && rayon::current_num_threads() > 1 {
            // Chunk rows so each task has a meaningful amount of work.
            let rows = data.len() / width;
            let rows_per_task = (16 * 1024 / width).max(1).min(rows.max(1));
            data.par_chunks_mut(width * rows_per_task)
                .enumerate()
                .for_each(|(ci, chunk)| {
                    for (ri, row) in chunk.chunks_mut(width).enumerate() {
                        f(ci * rows_per_task + ri, row);
                    }
                });
            return;
        }
    }
    for (y, row) in data.chunks_mut(width).enumerate() {
        f(y, row);
    }
}

/// Like [`for_rows`] for two equally-shaped buffers walked together.
#[inline]
pub(crate) fn for_rows2<T: Send, U: Sync, F>(dst: &mut [T], src: &[U], width: usize, f: F)
where
    F: Fn(usize, &mut [T], &[U]) + Sync + Send,
{
    if width == 0 {
        return;
    }
    debug_assert_eq!(dst.len(), src.len());
    #[cfg(feature = "parallel")]
    {
        use rayon::prelude::*;
        if dst.len() >= 64 * 1024 && rayon::current_num_threads() > 1 {
            let rows = dst.len() / width;
            let rows_per_task = (16 * 1024 / width).max(1).min(rows.max(1));
            let n = width * rows_per_task;
            dst.par_chunks_mut(n)
                .zip(src.par_chunks(n))
                .enumerate()
                .for_each(|(ci, (d, s))| {
                    for (ri, (dr, sr)) in d.chunks_mut(width).zip(s.chunks(width)).enumerate() {
                        f(ci * rows_per_task + ri, dr, sr);
                    }
                });
            return;
        }
    }
    for (y, (d, s)) in dst.chunks_mut(width).zip(src.chunks(width)).enumerate() {
        f(y, d, s);
    }
}
