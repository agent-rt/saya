use clap::{Parser, Subcommand};
use saya_core::{Database, SearchQuery};

#[derive(Parser)]
#[command(name = "saya", version, about = "Saya — local AI launcher & clipboard")]
struct Cli {
    /// Override the database path. Defaults to ~/Library/Application Support/Saya/saya.db
    #[arg(long, global = true)]
    db: Option<std::path::PathBuf>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Search clipboard history. With --semantic, fuses literal + vector lanes
    /// via RRF (requires the `embedding` cargo feature).
    Search {
        query: String,
        #[arg(short, long, default_value_t = 20)]
        limit: usize,
        #[arg(long, value_enum, default_value_t = OutputFormat::Json)]
        format: OutputFormat,
        /// Enable the vector lane. Loads the embedder on first query.
        #[arg(long)]
        semantic: bool,
    },
    /// Show runtime status: db path, entry count, vector coverage.
    Status,
    /// Backfill embeddings for entries that don't yet have a vector.
    /// Requires the `embedding` cargo feature.
    Reindex {
        /// Maximum entries to process this run.
        #[arg(short, long, default_value_t = 1000)]
        limit: usize,
        /// Batch size for embedder calls.
        #[arg(short, long, default_value_t = 16)]
        batch: usize,
    },
    /// Run the clipboard monitor in the foreground until Ctrl-C.
    Watch {
        /// Also embed each new entry and write a vector (requires `embedding`).
        #[arg(long)]
        embed: bool,
    },
    /// List installed applications (sorted alphabetically).
    Apps {
        #[arg(short, long, default_value_t = 200)]
        limit: usize,
    },
    /// Fuzzy-match an installed application by name and launch the best hit.
    Launch {
        query: String,
        /// Print top matches without launching.
        #[arg(long)]
        dry_run: bool,
        #[arg(short, long, default_value_t = 5)]
        limit: usize,
    },
    /// Dev helper: append a clipboard entry from stdin or argument.
    #[command(hide = true)]
    Add {
        /// Inline text. If omitted, reads from stdin.
        text: Option<String>,
    },
    /// Dev helper: embed text and print the resulting 384-d vector.
    /// Requires the `embedding` cargo feature.
    #[cfg(feature = "embedding")]
    #[command(hide = true)]
    Embed {
        text: String,
        /// Print full 384 values instead of head/tail preview.
        #[arg(long)]
        full: bool,
    },
}

#[derive(Clone, clap::ValueEnum)]
enum OutputFormat {
    Json,
    Tsv,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let db_path = cli.db.unwrap_or_else(saya_core::paths::default_db_path);
    let db = Database::open(&db_path)?;

    match cli.cmd {
        Cmd::Search { query, limit, format, semantic } => {
            #[cfg_attr(not(feature = "embedding"), allow(unused_mut))]
            let mut searcher = saya_core::search::Searcher::new(db.clone());
            if semantic {
                #[cfg(feature = "embedding")]
                {
                    searcher = searcher.with_embedder(saya_core::ai::EmbedderHandle::new());
                }
                #[cfg(not(feature = "embedding"))]
                anyhow::bail!("--semantic requires building with `--features embedding`");
            }
            let hits = searcher.search(&SearchQuery { text: query, limit })?;
            match format {
                OutputFormat::Json => println!("{}", serde_json::to_string(&hits)?),
                OutputFormat::Tsv => {
                    for h in hits {
                        println!(
                            "{}\t{:.4}\t{}",
                            h.id,
                            h.score,
                            h.content.replace('\n', "\\n")
                        );
                    }
                }
            }
        }
        Cmd::Status => {
            println!("db_path: {}", db_path.display());
            println!("entries: {}", db.count()?);
            let pending = db.entries_missing_vectors(10_000)?.len();
            println!("entries without vectors: {pending}");
        }
        Cmd::Reindex { limit, batch } => {
            #[cfg(feature = "embedding")]
            {
                let pending = db.entries_missing_vectors(limit)?;
                if pending.is_empty() {
                    println!("nothing to reindex");
                } else {
                    println!("backfilling {} entries (batch={batch})", pending.len());
                    let emb = saya_core::ai::EmbedderHandle::new();
                    let total = pending.len();
                    let mut done = 0usize;
                    for chunk in pending.chunks(batch) {
                        let texts: Vec<&str> = chunk.iter().map(|e| e.content.as_str()).collect();
                        let vecs = emb.embed(&texts)?;
                        for (entry, v) in chunk.iter().zip(vecs.iter()) {
                            db.upsert_vector(entry.id, v)?;
                        }
                        done += chunk.len();
                        eprintln!("  {done}/{total}");
                    }
                    println!("done");
                }
            }
            #[cfg(not(feature = "embedding"))]
            {
                let _ = (limit, batch);
                anyhow::bail!("reindex requires building with `--features embedding`");
            }
        }
        Cmd::Watch { embed } => {
            #[cfg(target_os = "macos")]
            {
                use saya_core::clipboard::ClipboardMonitor;
                let mut mon = if embed {
                    #[cfg(feature = "embedding")]
                    {
                        let emb = saya_core::ai::EmbedderHandle::new();
                        ClipboardMonitor::start_with_embedder(db.clone(), emb)
                    }
                    #[cfg(not(feature = "embedding"))]
                    anyhow::bail!("--embed requires building with `--features embedding`");
                } else {
                    ClipboardMonitor::start(db.clone())
                };
                println!(
                    "clipboard monitor running. db={} embed={} (Ctrl-C to stop)",
                    db_path.display(),
                    embed
                );
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;
                rt.block_on(async { tokio::signal::ctrl_c().await })?;
                mon.stop();
                println!("\nclipboard monitor stopped. captured {} total entries.", db.count()?);
            }
            #[cfg(not(target_os = "macos"))]
            {
                let _ = embed;
                anyhow::bail!("clipboard monitor is macOS-only in MVP");
            }
        }
        Cmd::Apps { limit } => {
            let idx = saya_core::launcher::LauncherIndex::build()?;
            for app in idx.apps().iter().take(limit) {
                println!("{}\t{}", app.name, app.path.display());
            }
            eprintln!("\ntotal: {} apps", idx.apps().len());
        }
        Cmd::Launch { query, dry_run, limit } => {
            let idx = saya_core::launcher::LauncherIndex::build()?;
            let mru = db.launch_history()?;
            let hits = idx.match_query(&query, limit, &mru);
            if hits.is_empty() {
                anyhow::bail!("no app matches {query:?}");
            }
            if dry_run {
                for h in &hits {
                    println!("{:>6}\t{}\t{}", h.score, h.app.name, h.app.path.display());
                }
            } else {
                let top = &hits[0].app;
                println!("launching: {} ({})", top.name, top.path.display());
                saya_core::launcher::launch(&top.path)?;
                let path_str = top.path.to_string_lossy().to_string();
                if let Err(e) = db.record_launch(&path_str) {
                    tracing::warn!(error = %e, "record_launch failed");
                }
            }
        }
        #[cfg(feature = "embedding")]
        Cmd::Embed { text, full } => {
            use saya_core::ai::EmbedderHandle;
            let handle = EmbedderHandle::new();
            let t0 = std::time::Instant::now();
            let vec = handle.embed_one(&text)?;
            let dt = t0.elapsed();
            let norm: f32 = vec.iter().map(|v| v * v).sum::<f32>().sqrt();
            eprintln!("dim={} elapsed={:?} l2_norm={:.4}", vec.len(), dt, norm);
            if full {
                println!("{}", serde_json::to_string(&vec)?);
            } else {
                let head: Vec<f32> = vec.iter().take(4).copied().collect();
                let tail: Vec<f32> = vec.iter().rev().take(4).rev().copied().collect();
                println!("head: {head:?}");
                println!("tail: {tail:?}");
            }
        }
        Cmd::Add { text } => {
            let content = match text {
                Some(s) => s,
                None => {
                    use std::io::Read;
                    let mut buf = String::new();
                    std::io::stdin().read_to_string(&mut buf)?;
                    buf.trim_end_matches('\n').to_string()
                }
            };
            match db.insert_entry(&content)? {
                Some(id) => println!("inserted id={id}"),
                None => println!("skipped (duplicate of most recent)"),
            }
        }
    }
    Ok(())
}
