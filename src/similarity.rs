//! Close-match suggestions with the Ratcliff/Obershelp scoring the suggestion lines are
//! defined by: longest matching blocks found recursively, ratio 2*M/T, ties broken by the
//! larger candidate.

use std::collections::HashMap;

struct Matcher<'b> {
    b: &'b [char],
    b2j: HashMap<char, Vec<usize>>,
    full_count: HashMap<char, usize>,
}

impl<'b> Matcher<'b> {
    fn new(b: &'b [char]) -> Self {
        let mut b2j: HashMap<char, Vec<usize>> = HashMap::new();
        for (index, &ch) in b.iter().enumerate() {
            b2j.entry(ch).or_default().push(index);
        }
        let mut full_count = HashMap::new();
        for &ch in b {
            *full_count.entry(ch).or_insert(0) += 1;
        }
        if b.len() >= 200 {
            let limit = b.len() / 100 + 1;
            b2j.retain(|_, positions| positions.len() <= limit);
        }
        Matcher { b, b2j, full_count }
    }

    fn longest(&self, a: &[char], alo: usize, ahi: usize, blo: usize, bhi: usize) -> (usize, usize, usize) {
        let (mut best_i, mut best_j, mut best_size) = (alo, blo, 0usize);
        let mut lengths: HashMap<usize, usize> = HashMap::new();
        for (i, ch) in a.iter().enumerate().take(ahi).skip(alo) {
            let mut next: HashMap<usize, usize> = HashMap::new();
            if let Some(positions) = self.b2j.get(ch) {
                for &j in positions {
                    if j < blo {
                        continue;
                    }
                    if j >= bhi {
                        break;
                    }
                    let k = if j > 0 {
                        lengths.get(&(j - 1)).copied().unwrap_or(0)
                    } else {
                        0
                    } + 1;
                    next.insert(j, k);
                    if k > best_size {
                        best_i = i + 1 - k;
                        best_j = j + 1 - k;
                        best_size = k;
                    }
                }
            }
            lengths = next;
        }
        while best_i > alo && best_j > blo && a[best_i - 1] == self.b[best_j - 1] {
            best_i -= 1;
            best_j -= 1;
            best_size += 1;
        }
        while best_i + best_size < ahi
            && best_j + best_size < bhi
            && a[best_i + best_size] == self.b[best_j + best_size]
        {
            best_size += 1;
        }
        (best_i, best_j, best_size)
    }

    fn matches(&self, a: &[char]) -> usize {
        let mut queue = vec![(0, a.len(), 0, self.b.len())];
        let mut total = 0;
        while let Some((alo, ahi, blo, bhi)) = queue.pop() {
            let (i, j, k) = self.longest(a, alo, ahi, blo, bhi);
            if k > 0 {
                total += k;
                if alo < i && blo < j {
                    queue.push((alo, i, blo, j));
                }
                if i + k < ahi && j + k < bhi {
                    queue.push((i + k, ahi, j + k, bhi));
                }
            }
        }
        total
    }

    fn quick_matches(&self, a: &[char]) -> usize {
        let mut available: HashMap<char, isize> = HashMap::new();
        let mut matches = 0;
        for ch in a {
            let count = match available.get(ch) {
                Some(&value) => value,
                None => *self.full_count.get(ch).unwrap_or(&0) as isize,
            };
            available.insert(*ch, count - 1);
            if count > 0 {
                matches += 1;
            }
        }
        matches
    }
}

fn ratio(matches: usize, length: usize) -> f64 {
    if length == 0 {
        1.0
    } else {
        2.0 * matches as f64 / length as f64
    }
}

/// Return up to `n` candidates whose similarity to `word` reaches `cutoff`, best first.
pub fn close_matches(word: &str, candidates: &[String], n: usize, cutoff: f64) -> Vec<String> {
    let b: Vec<char> = word.chars().collect();
    let matcher = Matcher::new(&b);
    let mut scored: Vec<(f64, &String)> = Vec::new();
    for candidate in candidates {
        let a: Vec<char> = candidate.chars().collect();
        let length = a.len() + b.len();
        if ratio(a.len().min(b.len()), length) >= cutoff && ratio(matcher.quick_matches(&a), length) >= cutoff {
            let score = ratio(matcher.matches(&a), length);
            if score >= cutoff {
                scored.push((score, candidate));
            }
        }
    }
    scored.sort_by(|left, right| right.0.total_cmp(&left.0).then_with(|| right.1.cmp(left.1)));
    scored
        .into_iter()
        .take(n)
        .map(|(_, candidate)| candidate.clone())
        .collect()
}
