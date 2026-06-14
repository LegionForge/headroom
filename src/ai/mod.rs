use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::json;

use crate::collect::SystemSnapshot;
use crate::config::Config;

const GB: f64 = 1_073_741_824.0;

pub async fn get_recommendations(snap: &SystemSnapshot, cfg: &Config) -> Result<String> {
    let prompt = build_prompt(snap, cfg);
    if cfg.ai.provider.is_openai_compat() {
        openai_compat_request(&prompt, cfg).await
    } else {
        claude_request(&prompt, cfg).await
    }
}

// ── Anthropic Claude ──────────────────────────────────────────────────────────

async fn claude_request(prompt: &str, cfg: &Config) -> Result<String> {
    let api_key = cfg
        .ai
        .api_key
        .as_deref()
        .context("No API key. Set ANTHROPIC_API_KEY in env or ai.api_key in config.toml.")?;

    let client = Client::new();
    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&json!({
            "model": cfg.ai.resolved_model(),
            "max_tokens": 4096,
            "messages": [{ "role": "user", "content": prompt }]
        }))
        .send()
        .await
        .context("Claude API request failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Claude API error {status}: {body}");
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .context("Failed to parse Claude response")?;
    Ok(body["content"][0]["text"]
        .as_str()
        .unwrap_or("No response text")
        .to_string())
}

// ── OpenAI-compatible (Ollama, LM Studio, vLLM, OpenAI) ──────────────────────

async fn openai_compat_request(prompt: &str, cfg: &Config) -> Result<String> {
    let base = cfg.ai.resolved_base_url().trim_end_matches('/');
    let url = format!("{base}/v1/chat/completions");
    let provider = &cfg.ai.provider;

    let client = Client::new();
    let mut req = client
        .post(&url)
        .header("content-type", "application/json")
        .json(&json!({
            "model": cfg.ai.resolved_model(),
            "messages": [{ "role": "user", "content": prompt }],
            "max_tokens": 4096,
            "stream": false,
        }));

    if let Some(key) = cfg.ai.api_key.as_deref() {
        if !key.is_empty() {
            req = req.header("Authorization", format!("Bearer {key}"));
        }
    }

    let resp = req
        .send()
        .await
        .with_context(|| format!("{provider} request to {url} failed — is it running?"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("{provider} API error {status}: {body}");
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .with_context(|| format!("Failed to parse {provider} response"))?;

    let msg = &body["choices"][0]["message"];

    // Some thinking models (Qwen3, DeepSeek-R1) put the answer in content after
    // </think> tags, or emit an empty content + separate reasoning_content field.
    let content = msg["content"].as_str().unwrap_or("");
    let reasoning = msg["reasoning_content"].as_str().unwrap_or("");

    let raw = if !content.is_empty() {
        content
    } else {
        reasoning
    };
    let result = strip_thinking(raw);

    if result.is_empty() {
        // Surface raw body so we can see what the model actually returned
        let debug = serde_json::to_string_pretty(&body).unwrap_or_else(|_| body.to_string());
        anyhow::bail!("Empty response from {provider}. Raw body:\n{debug}");
    }

    Ok(result.to_string())
}

/// Strips `<think>...</think>` prefixes emitted by reasoning models.
fn strip_thinking(s: &str) -> &str {
    if let Some(end) = s.find("</think>") {
        s[end + 8..].trim_start()
    } else {
        s
    }
}

// ── Prompt ────────────────────────────────────────────────────────────────────

fn build_prompt(snap: &SystemSnapshot, cfg: &Config) -> String {
    let mem = &snap.memory;
    let paging = &snap.paging;
    let profile = &cfg.system_profile;

    let proc_list: String = snap
        .top_processes
        .iter()
        .take(10)
        .map(|p| {
            format!(
                "  {:<30} pid {:>6}  virtual {:>6}  physical {:>6}  cpu {:.1}%",
                p.name,
                p.pid,
                fmt_bytes(p.virtual_bytes),
                fmt_bytes(p.memory_bytes),
                p.cpu_percent
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"You are a system performance expert. Analyze these live metrics and provide:
1. Root cause analysis — what is consuming commit and why allocation failures occur
2. Short-term fixes — actions that can be taken today (config changes, process restarts, pagefile tuning)
3. Long-term optimizations — architectural or workflow changes

System profile:
  CPU: {cpu}
  RAM: {ram_gb} GB installed
  GPU: {gpu}
  Use cases: {uses}
  OS: Windows 11

Memory snapshot ({ts}):
  Physical total:    {phys_total:.2} GB
  Physical used:     {phys_used:.2} GB
  Physical avail:    {phys_avail:.2} GB
  Cached/standby:    {cached:.2} GB

Commit charge (primary diagnostic):
  Committed:         {committed:.2} GB
  Commit limit:      {commit_limit:.2} GB  (RAM + pagefile ceiling)
  Commit pressure:   {commit_pct:.1}%

Kernel pools:
  Paged pool:        {paged:.3} GB
  Non-paged pool:    {nonpaged:.3} GB

Pagefile:
  Total:             {pf_total:.2} GB
  Used:              {pf_used:.2} GB
  Usage:             {pf_pct:.1}%

Top 10 processes by physical working set:
  (Note: virtual column includes Chromium-family VA reservations ~3.5T each — these are
   mostly uncommitted. Physical column is the actual RAM footprint.)
{proc_list}

Key observation: committed ({committed:.1} GB) vs physical in use ({phys_used:.1} GB).
Gap of {gap:.1} GB = virtual address space reserved but not backed by RAM or pagefile."#,
        cpu = if profile.cpu.is_empty() {
            "unspecified".into()
        } else {
            profile.cpu.clone()
        },
        ram_gb = profile.ram_gb,
        gpu = if profile.gpu.is_empty() {
            "unspecified".into()
        } else {
            profile.gpu.clone()
        },
        uses = if profile.use_cases.is_empty() {
            "development, gaming, media".into()
        } else {
            profile.use_cases.join(", ")
        },
        ts = snap.timestamp.format("%Y-%m-%d %H:%M:%S UTC"),
        phys_total = mem.total_bytes as f64 / GB,
        phys_used = mem.used_bytes as f64 / GB,
        phys_avail = mem.available_bytes as f64 / GB,
        cached = mem.cached_bytes as f64 / GB,
        committed = mem.committed_bytes as f64 / GB,
        commit_limit = mem.commit_limit_bytes as f64 / GB,
        commit_pct = mem.commit_ratio() * 100.0,
        paged = mem.paged_pool_bytes as f64 / GB,
        nonpaged = mem.non_paged_pool_bytes as f64 / GB,
        pf_total = paging.total_bytes as f64 / GB,
        pf_used = paging.used_bytes as f64 / GB,
        pf_pct = paging.usage_ratio() * 100.0,
        gap = (mem.committed_bytes.saturating_sub(mem.used_bytes)) as f64 / GB,
        proc_list = proc_list,
    )
}

pub fn fmt_bytes(bytes: u64) -> String {
    const TB: f64 = 1_099_511_627_776.0;
    const GB: f64 = 1_073_741_824.0;
    const MB: f64 = 1_048_576.0;
    let b = bytes as f64;
    if b >= TB {
        format!("{:.1}T", b / TB)
    } else if b >= GB {
        format!("{:.1}G", b / GB)
    } else if b >= MB {
        format!("{:.0}M", b / MB)
    } else {
        format!("{:.0}K", b / 1024.0)
    }
}
