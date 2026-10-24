//! A port of libc++'s unstable `std::sort` for a user-supplied comparator
//! (`__introsort` without the branchless/bitset variants), so that elements
//! comparing equal end up in exactly the order the reference library
//! produces. The algorithm is unchanged between LLVM 19 and libc++ 21.1.6,
//! the version the pathfinding library was built against.
//!
//! Part of the LLVM Project, under the Apache License v2.0 with LLVM
//! Exceptions (see `LICENSE-LLVM` in this crate).
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

/// `std::sort(v.begin(), v.end(), less)`.
pub(crate) fn sort<T: Copy, F: FnMut(&T, &T) -> bool>(v: &mut [T], mut less: F) {
    let n = v.len();
    let depth = 2 * log2i(n);
    introsort(v, 0, n, &mut less, depth as isize, true);
}

fn log2i(n: usize) -> usize {
    if n == 0 {
        0
    } else {
        (usize::BITS - 1 - n.leading_zeros()) as usize
    }
}

fn sort3<T: Copy, F: FnMut(&T, &T) -> bool>(
    v: &mut [T],
    x: usize,
    y: usize,
    z: usize,
    c: &mut F,
) -> u32 {
    let mut r = 0;
    if !c(&v[y], &v[x]) {
        if !c(&v[z], &v[y]) {
            return r;
        }
        v.swap(y, z);
        r = 1;
        if c(&v[y], &v[x]) {
            v.swap(x, y);
            r = 2;
        }
        return r;
    }
    if c(&v[z], &v[y]) {
        v.swap(x, z);
        return 1;
    }
    v.swap(x, y);
    r = 1;
    if c(&v[z], &v[y]) {
        v.swap(y, z);
        r = 2;
    }
    r
}

fn sort4<T: Copy, F: FnMut(&T, &T) -> bool>(
    v: &mut [T],
    x1: usize,
    x2: usize,
    x3: usize,
    x4: usize,
    c: &mut F,
) {
    sort3(v, x1, x2, x3, c);
    if c(&v[x4], &v[x3]) {
        v.swap(x3, x4);
        if c(&v[x3], &v[x2]) {
            v.swap(x2, x3);
            if c(&v[x2], &v[x1]) {
                v.swap(x1, x2);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn sort5<T: Copy, F: FnMut(&T, &T) -> bool>(
    v: &mut [T],
    x1: usize,
    x2: usize,
    x3: usize,
    x4: usize,
    x5: usize,
    c: &mut F,
) {
    sort4(v, x1, x2, x3, x4, c);
    if c(&v[x5], &v[x4]) {
        v.swap(x4, x5);
        if c(&v[x4], &v[x3]) {
            v.swap(x3, x4);
            if c(&v[x3], &v[x2]) {
                v.swap(x2, x3);
                if c(&v[x2], &v[x1]) {
                    v.swap(x1, x2);
                }
            }
        }
    }
}

fn insertion_sort<T: Copy, F: FnMut(&T, &T) -> bool>(
    v: &mut [T],
    first: usize,
    last: usize,
    c: &mut F,
) {
    if first == last {
        return;
    }
    let mut i = first + 1;
    while i != last {
        let mut j = i - 1;
        if c(&v[i], &v[j]) {
            let t = v[i];
            let mut k = j;
            j = i;
            loop {
                v[j] = v[k];
                j = k;
                if j == first {
                    break;
                }
                k -= 1;
                if !c(&t, &v[k]) {
                    break;
                }
            }
            v[j] = t;
        }
        i += 1;
    }
}

fn insertion_sort_unguarded<T: Copy, F: FnMut(&T, &T) -> bool>(
    v: &mut [T],
    first: usize,
    last: usize,
    c: &mut F,
) {
    if first == last {
        return;
    }
    let mut i = first + 1;
    while i != last {
        let mut j = i - 1;
        if c(&v[i], &v[j]) {
            let t = v[i];
            let mut k = j;
            j = i;
            loop {
                v[j] = v[k];
                j = k;
                k -= 1;
                if !c(&t, &v[k]) {
                    break;
                }
            }
            v[j] = t;
        }
        i += 1;
    }
}

fn insertion_sort_incomplete<T: Copy, F: FnMut(&T, &T) -> bool>(
    v: &mut [T],
    first: usize,
    last: usize,
    c: &mut F,
) -> bool {
    match last - first {
        0 | 1 => return true,
        2 => {
            if c(&v[last - 1], &v[first]) {
                v.swap(first, last - 1);
            }
            return true;
        }
        3 => {
            sort3(v, first, first + 1, last - 1, c);
            return true;
        }
        4 => {
            sort4(v, first, first + 1, first + 2, last - 1, c);
            return true;
        }
        5 => {
            sort5(v, first, first + 1, first + 2, first + 3, last - 1, c);
            return true;
        }
        _ => {}
    }
    let mut j = first + 2;
    sort3(v, first, first + 1, j, c);
    const LIMIT: u32 = 8;
    let mut count = 0;
    let mut i = j + 1;
    while i != last {
        if c(&v[i], &v[j]) {
            let t = v[i];
            let mut k = j;
            j = i;
            loop {
                v[j] = v[k];
                j = k;
                if j == first {
                    break;
                }
                k -= 1;
                if !c(&t, &v[k]) {
                    break;
                }
            }
            v[j] = t;
            count += 1;
            if count == LIMIT {
                i += 1;
                return i == last;
            }
        }
        j = i;
        i += 1;
    }
    true
}

fn partition_with_equals_on_right<T: Copy, F: FnMut(&T, &T) -> bool>(
    v: &mut [T],
    mut first: usize,
    mut last: usize,
    c: &mut F,
) -> (usize, bool) {
    let begin = first;
    let pivot = v[first];
    loop {
        first += 1;
        if !c(&v[first], &pivot) {
            break;
        }
    }
    if begin == first - 1 {
        while first < last {
            last -= 1;
            if c(&v[last], &pivot) {
                break;
            }
        }
    } else {
        loop {
            last -= 1;
            if c(&v[last], &pivot) {
                break;
            }
        }
    }
    let already_partitioned = first >= last;
    while first < last {
        v.swap(first, last);
        loop {
            first += 1;
            if !c(&v[first], &pivot) {
                break;
            }
        }
        loop {
            last -= 1;
            if c(&v[last], &pivot) {
                break;
            }
        }
    }
    let pivot_pos = first - 1;
    if begin != pivot_pos {
        v[begin] = v[pivot_pos];
    }
    v[pivot_pos] = pivot;
    (pivot_pos, already_partitioned)
}

fn partition_with_equals_on_left<T: Copy, F: FnMut(&T, &T) -> bool>(
    v: &mut [T],
    mut first: usize,
    mut last: usize,
    c: &mut F,
) -> usize {
    let begin = first;
    let pivot = v[first];
    if c(&pivot, &v[last - 1]) {
        loop {
            first += 1;
            if c(&pivot, &v[first]) {
                break;
            }
        }
    } else {
        loop {
            first += 1;
            if !(first < last && !c(&pivot, &v[first])) {
                break;
            }
        }
    }
    if first < last {
        loop {
            last -= 1;
            if !c(&pivot, &v[last]) {
                break;
            }
        }
    }
    while first < last {
        v.swap(first, last);
        loop {
            first += 1;
            if c(&pivot, &v[first]) {
                break;
            }
        }
        loop {
            last -= 1;
            if !c(&pivot, &v[last]) {
                break;
            }
        }
    }
    let pivot_pos = first - 1;
    if begin != pivot_pos {
        v[begin] = v[pivot_pos];
    }
    v[pivot_pos] = pivot;
    first
}

// ---- heap fallback (`__partial_sort(first, last, last)`) ----

fn sift_down<T: Copy, F: FnMut(&T, &T) -> bool>(
    v: &mut [T],
    first: usize,
    c: &mut F,
    len: isize,
    mut start: usize,
) {
    let mut child = (start - first) as isize;
    if len < 2 || (len - 2) / 2 < child {
        return;
    }
    child = 2 * child + 1;
    let mut child_i = first + child as usize;
    if (child + 1) < len && c(&v[child_i], &v[child_i + 1]) {
        child_i += 1;
        child += 1;
    }
    if c(&v[child_i], &v[start]) {
        return;
    }
    let top = v[start];
    loop {
        v[start] = v[child_i];
        start = child_i;
        if (len - 2) / 2 < child {
            break;
        }
        child = 2 * child + 1;
        child_i = first + child as usize;
        if (child + 1) < len && c(&v[child_i], &v[child_i + 1]) {
            child_i += 1;
            child += 1;
        }
        if c(&v[child_i], &top) {
            break;
        }
    }
    v[start] = top;
}

fn floyd_sift_down<T: Copy, F: FnMut(&T, &T) -> bool>(
    v: &mut [T],
    first: usize,
    c: &mut F,
    len: isize,
) -> usize {
    let mut hole = first;
    let mut child_i = first;
    let mut child: isize = 0;
    loop {
        child_i += (child + 1) as usize;
        child = 2 * child + 1;
        if (child + 1) < len && c(&v[child_i], &v[child_i + 1]) {
            child_i += 1;
            child += 1;
        }
        v[hole] = v[child_i];
        hole = child_i;
        if child > (len - 2) / 2 {
            return hole;
        }
    }
}

fn sift_up<T: Copy, F: FnMut(&T, &T) -> bool>(
    v: &mut [T],
    first: usize,
    mut last: usize,
    c: &mut F,
    mut len: isize,
) {
    if len > 1 {
        len = (len - 2) / 2;
        let mut ptr = first + len as usize;
        last -= 1;
        if c(&v[ptr], &v[last]) {
            let t = v[last];
            loop {
                v[last] = v[ptr];
                last = ptr;
                if len == 0 {
                    break;
                }
                len = (len - 1) / 2;
                ptr = first + len as usize;
                if !c(&v[ptr], &t) {
                    break;
                }
            }
            v[last] = t;
        }
    }
}

fn pop_heap<T: Copy, F: FnMut(&T, &T) -> bool>(
    v: &mut [T],
    first: usize,
    mut last: usize,
    c: &mut F,
    len: isize,
) {
    if len > 1 {
        let top = v[first];
        let mut hole = floyd_sift_down(v, first, c, len);
        last -= 1;
        if hole == last {
            v[hole] = top;
        } else {
            v[hole] = v[last];
            hole += 1;
            v[last] = top;
            sift_up(v, first, hole, c, (hole - first) as isize);
        }
    }
}

fn heap_sort<T: Copy, F: FnMut(&T, &T) -> bool>(v: &mut [T], first: usize, last: usize, c: &mut F) {
    if first == last {
        return;
    }
    // make_heap
    let n = (last - first) as isize;
    if n > 1 {
        let mut start = (n - 2) / 2;
        while start >= 0 {
            sift_down(v, first, c, n, first + start as usize);
            start -= 1;
        }
    }
    // (middle == last: nothing to select)
    // sort_heap
    let mut l = last;
    let mut n = (last - first) as isize;
    while n > 1 {
        pop_heap(v, first, l, c, n);
        l -= 1;
        n -= 1;
    }
}

fn introsort<T: Copy, F: FnMut(&T, &T) -> bool>(
    v: &mut [T],
    mut first: usize,
    mut last: usize,
    c: &mut F,
    mut depth: isize,
    mut leftmost: bool,
) {
    const LIMIT: usize = 24;
    const NINTHER_THRESHOLD: usize = 128;
    loop {
        let len = last - first;
        match len {
            0 | 1 => return,
            2 => {
                if c(&v[last - 1], &v[first]) {
                    v.swap(first, last - 1);
                }
                return;
            }
            3 => {
                sort3(v, first, first + 1, last - 1, c);
                return;
            }
            4 => {
                sort4(v, first, first + 1, first + 2, last - 1, c);
                return;
            }
            5 => {
                sort5(v, first, first + 1, first + 2, first + 3, last - 1, c);
                return;
            }
            _ => {}
        }
        if len < LIMIT {
            if leftmost {
                insertion_sort(v, first, last, c);
            } else {
                insertion_sort_unguarded(v, first, last, c);
            }
            return;
        }
        if depth == 0 {
            heap_sort(v, first, last, c);
            return;
        }
        depth -= 1;
        {
            let half_len = len / 2;
            if len > NINTHER_THRESHOLD {
                sort3(v, first, first + half_len, last - 1, c);
                sort3(v, first + 1, first + (half_len - 1), last - 2, c);
                sort3(v, first + 2, first + (half_len + 1), last - 3, c);
                sort3(
                    v,
                    first + (half_len - 1),
                    first + half_len,
                    first + (half_len + 1),
                    c,
                );
                v.swap(first, first + half_len);
            } else {
                sort3(v, first + half_len, first, last - 1, c);
            }
        }
        if !leftmost && !c(&v[first - 1], &v[first]) {
            first = partition_with_equals_on_left(v, first, last, c);
            continue;
        }
        let (i, already_partitioned) = partition_with_equals_on_right(v, first, last, c);
        if already_partitioned {
            let fs = insertion_sort_incomplete(v, first, i, c);
            if insertion_sort_incomplete(v, i + 1, last, c) {
                if fs {
                    return;
                }
                last = i;
                continue;
            } else if fs {
                first = i + 1;
                continue;
            }
        }
        introsort(v, first, i, c, depth, leftmost);
        leftmost = false;
        first = i + 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorts_like_a_sort() {
        let mut seed = 12345u64;
        for n in 0..600 {
            let mut v: Vec<(u32, u32)> = (0..n)
                .map(|i| {
                    seed = seed
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407);
                    (((seed >> 33) % 17) as u32, i as u32)
                })
                .collect();
            sort(&mut v, |a, b| a.0 < b.0);
            assert!(v.windows(2).all(|w| w[0].0 <= w[1].0));
        }
    }

    #[test]
    fn heap_fallback_sorts() {
        let mut v: Vec<i32> = (0..200).rev().collect();
        heap_sort(&mut v, 0, 200, &mut |a: &i32, b: &i32| a < b);
        assert!(v.windows(2).all(|w| w[0] <= w[1]));
    }
}
