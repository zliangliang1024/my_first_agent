use crate::config::Config;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Serialize, Deserialize, Clone)]
pub struct Memory {
    pub text: String,
    pub embedding: Vec<f32>,
}

// 1. 调用大模型把文本变成向量 (假设使用 OpenAI 兼容的 Embedding 接口)
pub async fn get_embedding(cfg: &Config, text: &str) -> Result<Vec<f32>> {
    if cfg.embedding_api_key.trim().is_empty() {
        anyhow::bail!("未配置 Embedding API Key");
    }

    let client = reqwest::Client::new();
    let body = json!({
        "model": "embedding-3", // ✅ 改为智谱模型
        "input": text,
        "encoding_format": "float"
    });

    let resp = client
        .post(&cfg.embedding_url)
        .bearer_auth(&cfg.embedding_api_key)
        .json(&body)
        .send()
        .await?;

    let data: Value = resp.json().await?;
    let embedding = data["data"][0]["embedding"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Embedding 解析失败"))?
        .iter()
        .map(|v| v.as_f64().unwrap_or(0.0) as f32)
        .collect();

    Ok(embedding)
}

// 2. 计算余弦相似度
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot / (norm_a * norm_b)
    }
}

// 3. 存入记忆库
pub fn save_memory(text: &str, embedding: Vec<f32>) {
    let mut memories: Vec<Memory> = crate::storage::load_json("memory.json").unwrap_or_default();
    memories.push(Memory {
        text: text.to_string(),
        embedding,
    });
    // 限制最多存 1000 条，防止文件太大
    if memories.len() > 1000 {
        memories.remove(0);
    }
    crate::storage::save_json("memory.json", &memories);
}

// 4. 检索最相关的 Top-K 记忆
pub fn retrieve_memory(query_embedding: &[f32], top_k: usize) -> Vec<String> {
    let memories: Vec<Memory> = crate::storage::load_json("memory.json").unwrap_or_default();

    let mut scored: Vec<(f32, &Memory)> = memories
        .iter()
        .map(|m| (cosine_similarity(query_embedding, &m.embedding), m))
        .collect();

    // 按相似度降序排序
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    // ✅ 加上这行，你会在终端看到所有历史记忆的匹配得分
    for (score, m) in &scored {
        println!(
            "[RAG] 相似度得分: {:.4} | 文本: {}",
            score,
            m.text.chars().take(20).collect::<String>()
        );
    }

    // 取前 K 个，且相似度必须大于 0.5 才算有效记忆
    scored
        .into_iter()
        .take(top_k)
        .filter(|(score, _)| *score > 0.2)
        .map(|(_, m)| m.text.clone())
        .collect()
}
