//! Bit-exact ports of .NET library behaviour that effects depend on.

/// `System.Random(seed)`: Knuth's subtractive generator, as used by .NET for
/// explicitly seeded instances.
pub struct DotNetRandom {
    seed_array: [i32; 56],
    inext: usize,
    inextp: usize,
}

const MBIG: i32 = i32::MAX;
const MSEED: i32 = 161803398;

impl DotNetRandom {
    pub fn new(seed: i32) -> Self {
        let mut sa = [0i32; 56];
        let subtraction = if seed == i32::MIN { i32::MAX } else { seed.abs() };
        let mut mj = MSEED - subtraction;
        sa[55] = mj;
        let mut mk = 1;
        let mut ii = 0usize;
        for _ in 1..55 {
            ii += 21;
            if ii >= 55 {
                ii -= 55;
            }
            sa[ii] = mk;
            mk = mj - mk;
            if mk < 0 {
                mk += MBIG;
            }
            mj = sa[ii];
        }
        for _ in 1..5 {
            for i in 1..56 {
                let mut n = i + 30;
                if n >= 55 {
                    n -= 55;
                }
                sa[i] = sa[i].wrapping_sub(sa[1 + n]);
                if sa[i] < 0 {
                    sa[i] += MBIG;
                }
            }
        }
        DotNetRandom { seed_array: sa, inext: 0, inextp: 21 }
    }

    fn internal_sample(&mut self) -> i32 {
        let mut a = self.inext + 1;
        if a >= 56 {
            a = 1;
        }
        let mut b = self.inextp + 1;
        if b >= 56 {
            b = 1;
        }
        let mut r = self.seed_array[a].wrapping_sub(self.seed_array[b]);
        if r == MBIG {
            r -= 1;
        }
        if r < 0 {
            r += MBIG;
        }
        self.seed_array[a] = r;
        self.inext = a;
        self.inextp = b;
        r
    }

    #[inline]
    pub fn next_double(&mut self) -> f64 {
        self.internal_sample() as f64 * (1.0 / MBIG as f64)
    }

    /// `Next(maxValue)`.
    #[inline]
    pub fn next_max(&mut self, max: i32) -> i32 {
        (self.next_double() * max as f64) as i32
    }
}

/// `List<T>.Sort(Comparison<T>)`: .NET's introspective sort. It is not
/// stable, so equal keys must land where .NET puts them.
pub fn introsort<T: Copy>(keys: &mut [T], cmp: &impl Fn(&T, &T) -> std::cmp::Ordering) {
    if keys.len() > 1 {
        let depth = 2 * ((keys.len() as u32).ilog2() as i32 + 1);
        intro(keys, depth, cmp);
    }
}

fn swap_if_greater<T: Copy>(k: &mut [T], cmp: &impl Fn(&T, &T) -> std::cmp::Ordering, i: usize, j: usize) {
    if cmp(&k[i], &k[j]).is_gt() {
        k.swap(i, j);
    }
}

fn intro<T: Copy>(keys: &mut [T], mut depth: i32, cmp: &impl Fn(&T, &T) -> std::cmp::Ordering) {
    let mut size = keys.len();
    while size > 1 {
        if size <= 16 {
            match size {
                2 => swap_if_greater(keys, cmp, 0, 1),
                3 => {
                    swap_if_greater(keys, cmp, 0, 1);
                    swap_if_greater(keys, cmp, 0, 2);
                    swap_if_greater(keys, cmp, 1, 2);
                }
                _ => insertion(&mut keys[..size], cmp),
            }
            return;
        }
        if depth == 0 {
            heap(&mut keys[..size], cmp);
            return;
        }
        depth -= 1;
        let p = partition(&mut keys[..size], cmp);
        intro(&mut keys[p + 1..size], depth, cmp);
        size = p;
    }
}

fn partition<T: Copy>(k: &mut [T], cmp: &impl Fn(&T, &T) -> std::cmp::Ordering) -> usize {
    let hi = k.len() - 1;
    let mid = hi >> 1;
    swap_if_greater(k, cmp, 0, mid);
    swap_if_greater(k, cmp, 0, hi);
    swap_if_greater(k, cmp, mid, hi);
    let pivot = k[mid];
    k.swap(mid, hi - 1);
    let (mut left, mut right) = (0usize, hi - 1);
    while left < right {
        loop {
            left += 1;
            if !cmp(&k[left], &pivot).is_lt() {
                break;
            }
        }
        loop {
            right -= 1;
            if !cmp(&pivot, &k[right]).is_lt() {
                break;
            }
        }
        if left >= right {
            break;
        }
        k.swap(left, right);
    }
    if left != hi - 1 {
        k.swap(left, hi - 1);
    }
    left
}

fn insertion<T: Copy>(k: &mut [T], cmp: &impl Fn(&T, &T) -> std::cmp::Ordering) {
    for i in 0..k.len() - 1 {
        let t = k[i + 1];
        let mut j = i as isize;
        while j >= 0 && cmp(&t, &k[j as usize]).is_lt() {
            k[j as usize + 1] = k[j as usize];
            j -= 1;
        }
        k[(j + 1) as usize] = t;
    }
}

fn heap<T: Copy>(k: &mut [T], cmp: &impl Fn(&T, &T) -> std::cmp::Ordering) {
    let n = k.len();
    for i in (1..=n >> 1).rev() {
        down_heap(k, i, n, cmp);
    }
    for i in (2..=n).rev() {
        k.swap(0, i - 1);
        down_heap(k, 1, i - 1, cmp);
    }
}

fn down_heap<T: Copy>(k: &mut [T], mut i: usize, n: usize, cmp: &impl Fn(&T, &T) -> std::cmp::Ordering) {
    let d = k[i - 1];
    while i <= n >> 1 {
        let mut child = 2 * i;
        if child < n && cmp(&k[child - 1], &k[child]).is_lt() {
            child += 1;
        }
        if !cmp(&d, &k[child - 1]).is_lt() {
            break;
        }
        k[i - 1] = k[child - 1];
        i = child;
    }
    k[i - 1] = d;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_known_values() {
        // Exact sequences are verified against .NET by the parity scripts.
        let mut r = DotNetRandom::new(0);
        let d = r.next_double();
        assert!((0.0..1.0).contains(&d));
    }

    #[test]
    fn introsort_sorts() {
        let mut v: Vec<i32> = (0..200).map(|i| (i * 7919) % 101).collect();
        introsort(&mut v, &|a, b| a.cmp(b));
        assert!(v.windows(2).all(|w| w[0] <= w[1]));
    }
}
