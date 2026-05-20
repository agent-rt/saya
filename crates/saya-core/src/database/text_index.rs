//! Tantivy BM25 index over clipboard entries, with a jieba-rs tokenizer
//! so Chinese / English / mixed text all segment sensibly.
//!
//! Layout:
//! - Schema: `id INTEGER (indexed/stored/fast)`, `content TEXT (jieba-tokenized)`
//! - Writer: serialized via Mutex; `commit()` after every write so reads see
//!   the new doc immediately. Acceptable for clipboard cadence (~writes/sec
//!   at most). If we ever batch we can buffer commits.
//! - Reader: `ReloadPolicy::OnCommit`, auto-refreshes when a commit lands.
//! - Persisted on disk for file-backed DBs; in-RAM index for tests.

use std::path::Path;
use std::sync::{Arc, Mutex};

use jieba_rs::Jieba;
use tantivy::collector::TopDocs;
use tantivy::directory::MmapDirectory;
use tantivy::query::QueryParser;
use tantivy::schema::{
    FAST, Field, INDEXED, IndexRecordOption, STORED, Schema, TextFieldIndexing, TextOptions, Value,
};
use tantivy::tokenizer::{LowerCaser, TextAnalyzer, Token, TokenStream, Tokenizer};
use tantivy::{Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term, doc};

const TOKENIZER_NAME: &str = "jieba";
const WRITER_HEAP_BYTES: usize = 50_000_000;

pub struct TextIndex {
    index: Index,
    writer: Mutex<IndexWriter>,
    reader: IndexReader,
    id_field: Field,
    content_field: Field,
}

impl TextIndex {
    pub fn open(dir: &Path) -> crate::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let schema = build_schema();
        let dir_handle = MmapDirectory::open(dir)
            .map_err(|e| crate::Error::Other(format!("tantivy dir: {e}")))?;
        let index = Index::open_or_create(dir_handle, schema.clone())
            .map_err(|e| crate::Error::Other(format!("tantivy open: {e}")))?;
        Self::finish(index, schema)
    }

    pub fn open_in_memory() -> crate::Result<Self> {
        let schema = build_schema();
        let index = Index::create_in_ram(schema.clone());
        Self::finish(index, schema)
    }

    fn finish(index: Index, schema: Schema) -> crate::Result<Self> {
        index
            .tokenizers()
            .register(TOKENIZER_NAME, jieba_analyzer());
        let id_field = schema.get_field("id").expect("id field");
        let content_field = schema.get_field("content").expect("content field");
        let writer = index
            .writer(WRITER_HEAP_BYTES)
            .map_err(|e| crate::Error::Other(format!("tantivy writer: {e}")))?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .map_err(|e| crate::Error::Other(format!("tantivy reader: {e}")))?;
        Ok(Self {
            index,
            writer: Mutex::new(writer),
            reader,
            id_field,
            content_field,
        })
    }

    pub fn insert(&self, id: i64, content: &str) -> crate::Result<()> {
        let mut writer = self.writer.lock().expect("text index writer");
        writer
            .add_document(doc!(
                self.id_field => id,
                self.content_field => content,
            ))
            .map_err(|e| crate::Error::Other(format!("tantivy add: {e}")))?;
        writer
            .commit()
            .map_err(|e| crate::Error::Other(format!("tantivy commit: {e}")))?;
        Ok(())
    }

    pub fn delete(&self, id: i64) -> crate::Result<()> {
        let mut writer = self.writer.lock().expect("text index writer");
        writer.delete_term(Term::from_field_i64(self.id_field, id));
        writer
            .commit()
            .map_err(|e| crate::Error::Other(format!("tantivy commit: {e}")))?;
        Ok(())
    }

    /// BM25 search returning (entry_id, score) sorted by descending score.
    pub fn search(&self, query: &str, limit: usize) -> crate::Result<Vec<(i64, f32)>> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let _ = self.reader.reload();
        let searcher = self.reader.searcher();
        let mut parser = QueryParser::for_index(&self.index, vec![self.content_field]);
        parser.set_conjunction_by_default();
        // First try a conjunctive query; if it returns nothing, fall back to OR.
        let parsed = parser.parse_query_lenient(query).0;
        // Tantivy 0.26: TopDocs is a builder; call order_by_score() to get the
        // concrete Collector that returns BM25 scores.
        let top = searcher
            .search(
                parsed.as_ref(),
                &TopDocs::with_limit(limit).order_by_score(),
            )
            .map_err(|e| crate::Error::Other(format!("tantivy search: {e}")))?;
        let mut out = Vec::with_capacity(top.len());
        for (score, addr) in top {
            let doc: TantivyDocument = searcher
                .doc(addr)
                .map_err(|e| crate::Error::Other(format!("tantivy doc: {e}")))?;
            if let Some(id) = doc.get_first(self.id_field).and_then(|v| v.as_i64()) {
                out.push((id, score));
            }
        }
        Ok(out)
    }
}

fn build_schema() -> Schema {
    let mut sb = Schema::builder();
    sb.add_i64_field("id", INDEXED | STORED | FAST);
    let content_options = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer(TOKENIZER_NAME)
            .set_index_option(IndexRecordOption::WithFreqsAndPositions),
    );
    sb.add_text_field("content", content_options);
    sb.build()
}

fn jieba_analyzer() -> TextAnalyzer {
    TextAnalyzer::builder(JiebaTokenizer::default())
        .filter(LowerCaser)
        .build()
}

#[derive(Clone)]
struct JiebaTokenizer {
    jieba: Arc<Jieba>,
}

impl Default for JiebaTokenizer {
    fn default() -> Self {
        Self {
            jieba: Arc::new(Jieba::new()),
        }
    }
}

struct JiebaTokenStream {
    tokens: Vec<Token>,
    idx: usize,
}

impl Tokenizer for JiebaTokenizer {
    type TokenStream<'a> = JiebaTokenStream;

    fn token_stream<'a>(&'a mut self, text: &'a str) -> Self::TokenStream<'a> {
        let jieba_tokens = self
            .jieba
            .tokenize(text, jieba_rs::TokenizeMode::Search, true);
        let tokens = jieba_tokens
            .into_iter()
            .enumerate()
            .map(|(i, t)| Token {
                offset_from: t.start,
                offset_to: t.end,
                position: i,
                text: t.word.to_string(),
                position_length: 1,
            })
            .collect();
        JiebaTokenStream { tokens, idx: 0 }
    }
}

impl TokenStream for JiebaTokenStream {
    fn advance(&mut self) -> bool {
        if self.idx < self.tokens.len() {
            self.idx += 1;
            true
        } else {
            false
        }
    }
    fn token(&self) -> &Token {
        &self.tokens[self.idx - 1]
    }
    fn token_mut(&mut self) -> &mut Token {
        &mut self.tokens[self.idx - 1]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_search() {
        // BM25 over jieba tokens matches whole terms (no English stemming).
        // "brown" hits "brown" tokens, but "brownish" is a different token.
        let idx = TextIndex::open_in_memory().unwrap();
        idx.insert(1, "the quick brown fox jumps").unwrap();
        idx.insert(2, "lazy dog under the bridge").unwrap();
        idx.insert(3, "another brown bear").unwrap();
        let hits = idx.search("brown", 10).unwrap();
        let ids: Vec<i64> = hits.iter().map(|(id, _)| *id).collect();
        assert!(ids.contains(&1), "doc 1 missing: {ids:?}");
        assert!(ids.contains(&3), "doc 3 missing: {ids:?}");
        assert!(!ids.contains(&2), "doc 2 should not match: {ids:?}");
    }

    #[test]
    fn chinese_search_segments_with_jieba() {
        let idx = TextIndex::open_in_memory().unwrap();
        idx.insert(1, "今天天气真好,我们去公园散步").unwrap();
        idx.insert(2, "明天的会议被推迟了").unwrap();
        idx.insert(3, "公园里有很多人在跑步").unwrap();
        let hits = idx.search("公园", 10).unwrap();
        let ids: Vec<i64> = hits.iter().map(|(id, _)| *id).collect();
        assert!(ids.contains(&1));
        assert!(ids.contains(&3));
        assert!(!ids.contains(&2));
    }

    #[test]
    fn delete_removes_from_search_results() {
        let idx = TextIndex::open_in_memory().unwrap();
        idx.insert(1, "hello world").unwrap();
        idx.insert(2, "hello universe").unwrap();
        idx.delete(1).unwrap();
        let hits = idx.search("hello", 10).unwrap();
        let ids: Vec<i64> = hits.iter().map(|(id, _)| *id).collect();
        assert!(!ids.contains(&1));
        assert!(ids.contains(&2));
    }
}
