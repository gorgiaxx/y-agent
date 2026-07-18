//! Deterministic lexical ranking for unified capability discovery.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use y_knowledge::{AutoTokenizer, Bm25Index, Tokenizer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CapabilityKind {
    Tool,
    Skill,
    Agent,
    Workflow,
}

#[derive(Debug, Clone)]
pub(crate) struct CapabilityDocument {
    pub kind: CapabilityKind,
    pub id: String,
    pub name: String,
    pub description: String,
    pub aliases: Vec<String>,
    pub keywords: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CapabilityMatchReason {
    ExactId,
    ExactName,
    Bm25,
}

impl CapabilityMatchReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ExactId => "exact_id",
            Self::ExactName => "exact_name",
            Self::Bm25 => "bm25",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CapabilitySearchHit {
    pub kind: CapabilityKind,
    pub id: String,
    pub name: String,
    pub score: u32,
    pub reason: CapabilityMatchReason,
}

#[derive(Debug, Default)]
struct CapabilityTokenizer {
    natural_language: AutoTokenizer,
}

impl Tokenizer for CapabilityTokenizer {
    fn tokenize(&self, text: &str) -> Vec<String> {
        let identifier_text = split_identifier_fragments(text).join(" ");
        self.natural_language.tokenize(&identifier_text)
    }
}

pub(crate) struct CapabilitySearchIndex {
    documents: HashMap<String, CapabilityDocument>,
    bm25: Bm25Index<CapabilityTokenizer>,
}

impl CapabilitySearchIndex {
    pub fn build(mut documents: Vec<CapabilityDocument>) -> Self {
        documents.sort_by(compare_documents);
        let mut bm25 = Bm25Index::new(CapabilityTokenizer::default());
        let mut by_key = HashMap::with_capacity(documents.len());
        for document in documents {
            let key = document_key(document.kind, &document.id);
            let content = weighted_document_text(&document);
            bm25.add(&key, &content);
            by_key.insert(key, document);
        }
        Self {
            documents: by_key,
            bm25,
        }
    }

    pub fn search(&self, query: &str, limit: usize) -> Vec<CapabilitySearchHit> {
        if limit == 0 {
            return Vec::new();
        }
        let normalized_query = normalize_exact(query);
        if normalized_query.is_empty() {
            return Vec::new();
        }

        let mut hits = self.exact_hits(&normalized_query);
        let seen: HashSet<String> = hits
            .iter()
            .map(|hit| document_key(hit.kind, &hit.id))
            .collect();
        let raw_results = self.bm25.search(query, self.documents.len());
        let max_raw_score = raw_results
            .iter()
            .map(|result| result.score)
            .fold(0.0_f64, f64::max);
        let query_tokens: HashSet<String> = identifier_tokens(query).into_iter().collect();

        hits.extend(raw_results.into_iter().filter_map(|result| {
            if seen.contains(&result.chunk_id) {
                return None;
            }
            let document = self.documents.get(&result.chunk_id)?;
            Some(CapabilitySearchHit {
                kind: document.kind,
                id: document.id.clone(),
                name: document.name.clone(),
                score: calibrated_bm25_score(result.score, max_raw_score, &query_tokens, document),
                reason: CapabilityMatchReason::Bm25,
            })
        }));
        hits.sort_by(compare_hits);
        hits.truncate(limit);
        hits
    }

    fn exact_hits(&self, normalized_query: &str) -> Vec<CapabilitySearchHit> {
        self.documents
            .values()
            .filter_map(|document| {
                let reason = if normalize_exact(&document.name) == normalized_query
                    || document
                        .aliases
                        .iter()
                        .any(|alias| normalize_exact(alias) == normalized_query)
                {
                    CapabilityMatchReason::ExactName
                } else if normalize_exact(&document.id) == normalized_query {
                    CapabilityMatchReason::ExactId
                } else {
                    return None;
                };
                Some(CapabilitySearchHit {
                    kind: document.kind,
                    id: document.id.clone(),
                    name: document.name.clone(),
                    score: 10_000,
                    reason,
                })
            })
            .collect()
    }
}

fn calibrated_bm25_score(
    raw_score: f64,
    max_raw_score: f64,
    query_tokens: &HashSet<String>,
    document: &CapabilityDocument,
) -> u32 {
    let normalized_bm25 = if max_raw_score > 0.0 {
        (raw_score / max_raw_score * 7_000.0).round() as u32
    } else {
        0
    };
    let document_tokens: HashSet<String> = identifier_tokens(&weighted_document_text(document))
        .into_iter()
        .collect();
    let matched = query_tokens.intersection(&document_tokens).count();
    let coverage = if query_tokens.is_empty() {
        0
    } else {
        u32::try_from(matched)
            .unwrap_or(u32::MAX)
            .saturating_mul(2_000)
            / u32::try_from(query_tokens.len()).unwrap_or(u32::MAX).max(1)
    };
    let name_tokens: HashSet<String> = identifier_tokens(&document.name).into_iter().collect();
    let name_overlap = u32::try_from(query_tokens.intersection(&name_tokens).count())
        .unwrap_or(u32::MAX)
        .saturating_mul(250)
        .min(500);
    normalized_bm25
        .saturating_add(coverage)
        .saturating_add(name_overlap)
        .clamp(1, 9_499)
}

fn weighted_document_text(document: &CapabilityDocument) -> String {
    let mut fields = Vec::new();
    fields.extend(std::iter::repeat_n(document.name.as_str(), 4));
    fields.extend(std::iter::repeat_n(document.id.as_str(), 3));
    for alias in &document.aliases {
        fields.extend(std::iter::repeat_n(alias.as_str(), 3));
    }
    for keyword in &document.keywords {
        fields.extend(std::iter::repeat_n(keyword.as_str(), 2));
    }
    fields.push(&document.description);
    fields.join(" ")
}

fn document_key(kind: CapabilityKind, id: &str) -> String {
    format!("{}:{id}", kind_order(kind))
}

fn compare_documents(left: &CapabilityDocument, right: &CapabilityDocument) -> Ordering {
    kind_order(left.kind)
        .cmp(&kind_order(right.kind))
        .then_with(|| left.name.cmp(&right.name))
        .then_with(|| left.id.cmp(&right.id))
}

fn compare_hits(left: &CapabilitySearchHit, right: &CapabilitySearchHit) -> Ordering {
    right
        .score
        .cmp(&left.score)
        .then_with(|| kind_order(left.kind).cmp(&kind_order(right.kind)))
        .then_with(|| left.name.cmp(&right.name))
        .then_with(|| left.id.cmp(&right.id))
}

fn kind_order(kind: CapabilityKind) -> u8 {
    match kind {
        CapabilityKind::Tool => 0,
        CapabilityKind::Skill => 1,
        CapabilityKind::Agent => 2,
        CapabilityKind::Workflow => 3,
    }
}

fn normalize_exact(value: &str) -> String {
    value.trim().to_lowercase()
}

pub(crate) fn identifier_tokens(value: &str) -> Vec<String> {
    let identifier_text = split_identifier_fragments(value).join(" ");
    AutoTokenizer::new().tokenize(&identifier_text)
}

fn split_identifier_fragments(value: &str) -> Vec<String> {
    let characters: Vec<char> = value.chars().collect();
    let mut fragments = Vec::new();
    let mut current = String::new();

    for (index, character) in characters.iter().copied().enumerate() {
        if !character.is_alphanumeric() {
            push_fragment(&mut fragments, &mut current);
            continue;
        }
        let previous = index
            .checked_sub(1)
            .and_then(|position| characters.get(position));
        let next = characters.get(index + 1);
        let starts_word = character.is_uppercase()
            && previous.is_some_and(|value| {
                value.is_lowercase()
                    || value.is_numeric()
                    || (value.is_uppercase() && next.is_some_and(|value| value.is_lowercase()))
            });
        if starts_word {
            push_fragment(&mut fragments, &mut current);
        }
        current.push(character);
    }
    push_fragment(&mut fragments, &mut current);
    fragments
        .into_iter()
        .map(|fragment| fragment.to_lowercase())
        .filter(|fragment| !fragment.is_empty())
        .collect()
}

fn push_fragment(fragments: &mut Vec<String>, current: &mut String) {
    if !current.is_empty() {
        fragments.push(std::mem::take(current));
    }
}

#[cfg(test)]
mod tests {
    use super::{
        identifier_tokens, CapabilityDocument, CapabilityKind, CapabilityMatchReason,
        CapabilitySearchIndex,
    };

    fn document(
        kind: CapabilityKind,
        id: &str,
        name: &str,
        description: &str,
        keywords: &[&str],
    ) -> CapabilityDocument {
        CapabilityDocument {
            kind,
            id: id.to_string(),
            name: name.to_string(),
            description: description.to_string(),
            aliases: Vec::new(),
            keywords: keywords.iter().map(|value| (*value).to_string()).collect(),
        }
    }

    #[test]
    fn identifier_tokenizer_splits_common_code_naming_styles() {
        assert_eq!(
            identifier_tokens("HTTPServer file_read error-handler XMLHttpRequest"),
            vec!["http", "server", "file", "read", "error", "handler", "xml", "http", "request"]
        );
    }

    #[test]
    fn exact_name_outranks_lexically_similar_documents() {
        let index = CapabilitySearchIndex::build(vec![
            document(
                CapabilityKind::Skill,
                "skill-1",
                "file-reader-advice",
                "Advice for selecting file readers",
                &["file", "read"],
            ),
            document(
                CapabilityKind::Tool,
                "FileRead",
                "FileRead",
                "Read file contents",
                &["path"],
            ),
        ]);

        let results = index.search("FileRead", 10);

        assert_eq!(results[0].kind, CapabilityKind::Tool);
        assert_eq!(results[0].id, "FileRead");
        assert_eq!(results[0].score, 10_000);
        assert_eq!(results[0].reason, CapabilityMatchReason::ExactName);
    }

    #[test]
    fn exact_bare_alias_uses_the_exact_match_fast_path() {
        let mut tool = document(
            CapabilityKind::Tool,
            "mcp_github_search_repos",
            "mcp_github_search_repos",
            "Search GitHub repositories",
            &["github", "repository"],
        );
        tool.aliases.push("search_repos".to_string());
        let index = CapabilitySearchIndex::build(vec![tool]);

        let results = index.search("search_repos", 10);

        assert_eq!(results[0].id, "mcp_github_search_repos");
        assert_eq!(results[0].score, 10_000);
        assert_eq!(results[0].reason, CapabilityMatchReason::ExactName);
    }

    #[test]
    fn bm25_finds_a_needle_in_unrelated_capabilities() {
        let mut documents = (0..30)
            .map(|index| {
                document(
                    CapabilityKind::Tool,
                    &format!("generic-{index}"),
                    &format!("GenericTool{index}"),
                    "General formatting and text conversion helper",
                    &["text", "format"],
                )
            })
            .collect::<Vec<_>>();
        documents.push(document(
            CapabilityKind::Tool,
            "RepositoryIssueLookup",
            "RepositoryIssueLookup",
            "Search repository issue tracker entries by label and state",
            &["repository", "issues", "label", "state"],
        ));
        let index = CapabilitySearchIndex::build(documents);

        let results = index.search("find repository issues by label", 5);

        assert_eq!(results[0].id, "RepositoryIssueLookup");
        assert!(results[0].score > 0);
        assert_eq!(results[0].reason, CapabilityMatchReason::Bm25);
    }

    #[test]
    fn equal_scores_have_a_stable_name_and_id_order() {
        let index = CapabilitySearchIndex::build(vec![
            document(
                CapabilityKind::Tool,
                "z-id",
                "Zulu",
                "shared capability words",
                &[],
            ),
            document(
                CapabilityKind::Tool,
                "a-id",
                "Alpha",
                "shared capability words",
                &[],
            ),
        ]);

        let first = index.search("shared capability", 10);
        let second = index.search("shared capability", 10);

        assert_eq!(first, second);
        assert_eq!(first[0].name, "Alpha");
        assert_eq!(first[1].name, "Zulu");
    }

    #[test]
    fn exact_cross_type_identifier_uses_the_shared_score_scale() {
        let index = CapabilitySearchIndex::build(vec![
            document(
                CapabilityKind::Tool,
                "ReleaseNotes",
                "ReleaseNotes",
                "Prepare output for the release pipeline",
                &["release", "pipeline"],
            ),
            document(
                CapabilityKind::Workflow,
                "release-pipeline",
                "Release Pipeline",
                "Build, verify, and publish a release",
                &["build", "publish"],
            ),
        ]);

        let results = index.search("release-pipeline", 10);

        assert_eq!(results[0].kind, CapabilityKind::Workflow);
        assert_eq!(results[0].score, 10_000);
    }
}
