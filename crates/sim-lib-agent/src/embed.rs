use sim_cookbook::fnv1a64;

pub const EMBED_DIM: usize = 256;

pub fn embed(text: &str) -> [f32; EMBED_DIM] {
    let mut vector = [0.0f32; EMBED_DIM];
    for token in text
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|token| !token.is_empty())
    {
        let index = stable_hash(&token.to_ascii_lowercase()) % EMBED_DIM;
        vector[index] += 1.0;
    }
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in &mut vector {
            *value /= norm;
        }
    }
    vector
}

pub fn cosine(a: &[f32; EMBED_DIM], b: &[f32; EMBED_DIM]) -> f32 {
    a.iter()
        .zip(b.iter())
        .map(|(left, right)| left * right)
        .sum()
}

fn stable_hash(text: &str) -> usize {
    let mut bytes = Vec::with_capacity(text.len() + 1);
    bytes.extend_from_slice(text.as_bytes());
    bytes.push(0xff);
    fnv1a64(&bytes) as usize
}

#[cfg(test)]
mod tests {
    use super::{cosine, embed};

    #[test]
    fn embedding_is_deterministic_and_normalized() {
        let first = embed("SIM vectors stay deterministic");
        let second = embed("SIM vectors stay deterministic");
        assert_eq!(first, second);
        let norm = cosine(&first, &first);
        assert!((norm - 1.0).abs() < 0.0001);
    }

    #[test]
    fn embedding_of_empty_text_is_zero_vector() {
        let empty = embed("   ");
        assert_eq!(cosine(&empty, &empty), 0.0);
    }
}
