# Knowledge Base

The knowledge base supports **multi-level chunking** (L0 summary, L1 sections, L2 paragraphs) and **hybrid retrieval** (BM25 keyword search + vector semantic search).

## Keyword-Only (No External Services)

```toml
# config/knowledge.toml
embedding_enabled = false
retrieval_strategy = "keyword"
```

This mode requires no external services and works out of the box.

## Full Semantic / Hybrid Search

### Step 1 -- Start Qdrant

```bash
# Docker
docker run -p 6333:6333 -p 6334:6334 qdrant/qdrant:v1.8.4

# Or via docker-compose (includes PostgreSQL + Qdrant)
docker compose up -d qdrant
```

### Step 2 -- Configure Embedding

```toml
# config/knowledge.toml
embedding_enabled = true
embedding_model = "text-embedding-3-small"
embedding_dimensions = 1536
embedding_base_url = "https://api.openai.com/v1"
embedding_api_key_env = "OPENAI_API_KEY"
embedding_max_tokens = 8192

retrieval_strategy = "hybrid"   # "hybrid" | "semantic" | "keyword"
bm25_weight = 1.0
vector_weight = 1.0
```

### Alternative Embedding Providers

Any **OpenAI-compatible** `/v1/embeddings` endpoint works:

```toml
# Ollama local
embedding_model = "nomic-embed-text"
embedding_dimensions = 768
embedding_base_url = "http://localhost:11434/v1"
embedding_api_key = "not-needed"
embedding_max_tokens = 512

# Azure OpenAI
embedding_model = "text-embedding-3-small"
embedding_dimensions = 1536
embedding_base_url = "https://your-resource.openai.azure.com/openai/deployments/text-embedding-3-small"
embedding_api_key_env = "AZURE_EMBEDDING_KEY"
```

### Step 3 -- Configure Qdrant

```bash
export Y_QDRANT_URL=http://localhost:6334
# Or set in docker-compose (pre-wired)
```

## Usage

### Via GUI

1. Open the **Knowledge** tab in the sidebar
2. Click `+` to create a collection
3. Click **Import** to add files (`.md`, `.txt`, `.pdf`, `.rs`, `.py`, `.js`, `.ts`, `.toml`, `.yaml`, `.json`, `.html`, `.csv`, and more) or entire folders
4. Attach collections to a chat via the knowledge button in the input toolbar

### Via CLI

```bash
y-agent knowledge ingest --file docs/guide.md --collection project-docs
y-agent knowledge search "how does the auth module work"
```

## Chunking Configuration

```toml
# config/knowledge.toml
l0_max_tokens = 200       # L0: document summary
l1_max_tokens = 500       # L1: section overviews
l2_max_tokens = 450       # L2: paragraph chunks (retrieval source)

max_chunks_per_entry = 5000
min_similarity_threshold = 0.65
```

## Retrieval Strategies

| Strategy | Description |
|----------|-------------|
| `keyword` | BM25 keyword matching only. No external services needed. |
| `semantic` | Vector similarity search via Qdrant. Requires embedding model + Qdrant. |
| `hybrid` | Combines BM25 + vector search with configurable weights. Best accuracy. |
