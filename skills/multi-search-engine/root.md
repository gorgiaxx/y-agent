# Multi Search Engine

## Purpose
Guide LLMs in constructing effective search queries across 17 search engines (8 domestic CN + 9 international) without API keys. Provides URL templates, advanced operators, and search strategies.

## Search Engine Quick Reference

### Domestic Engines (CN)
| Engine | URL Template | Best For |
|--------|--------------|----------|
| Baidu | `https://www.baidu.com/s?wd={keyword}` | Chinese content |
| Bing CN | `https://cn.bing.com/search?q={keyword}&ensearch=0` | Chinese results |
| Bing INT | `https://cn.bing.com/search?q={keyword}&ensearch=1` | English results from CN |
| 360 | `https://www.so.com/s?q={keyword}` | Alternative CN search |
| Sogou | `https://sogou.com/web?query={keyword}` | WeChat content gateway |
| WeChat | `https://wx.sogou.com/weixin?type=2&query={keyword}` | WeChat articles |
| Toutiao | `https://so.toutiao.com/search?keyword={keyword}` | News, trending topics |
| Jisilu | `https://www.jisilu.cn/explore/?keyword={keyword}` | Finance, investment |

### International Engines
| Engine | URL Template | Best For |
|--------|--------------|----------|
| Google | `https://www.google.com/search?q={keyword}` | Comprehensive results |
| Google HK | `https://www.google.com.hk/search?q={keyword}` | CN-accessible Google |
| DuckDuckGo | `https://duckduckgo.com/html/?q={keyword}` | Privacy, Bangs shortcuts |
| Yahoo | `https://search.yahoo.com/search?p={keyword}` | Alternative index |
| Startpage | `https://www.startpage.com/sp/search?query={keyword}` | Google results + privacy |
| Brave | `https://search.brave.com/search?q={keyword}` | Independent index |
| Ecosia | `https://www.ecosia.org/search?q={keyword}` | Eco-friendly |
| Qwant | `https://www.qwant.com/?q={keyword}` | EU GDPR compliant |
| WolframAlpha | `https://www.wolframalpha.com/input?i={keyword}` | Knowledge computation |

## Core Search Rules

### Rule: Basic Search Construction
**Pattern**: User requests web search
**Action**: 
1. URL-encode the keyword (replace spaces with `+` or `%20`)
2. Construct URL using appropriate engine template
3. Use `web_fetch` tool to retrieve results
**Example**: For "machine learning tutorial" on Google → `https://www.google.com/search?q=machine+learning+tutorial`

### Rule: Engine Selection
**Pattern**: User has specific search intent
**Action**: Select engine based on intent:
- Privacy-sensitive → DuckDuckGo, Startpage, Brave
- Chinese content → Baidu, Bing CN, WeChat
- Academic → Google Scholar (`https://scholar.google.com/scholar?q={keyword}`)
- Knowledge computation → WolframAlpha
- Real-time news → Google News (`&tbm=nws`), Brave News (`&source=news`)

### Essential Operators
| Operator | Syntax | Purpose |
|----------|--------|---------|
| `site:` | `site:github.com python` | Search within specific domain |
| `filetype:` | `filetype:pdf report` | Find specific file types |
| `""` | `"exact phrase"` | Exact match |
| `-` | `python -snake` | Exclude term |
| `OR` | `cat OR dog` | Either term |

### Time Filters (Google)
| Parameter | Time Range |
|-----------|------------|
| `tbs=qdr:h` | Past hour |
| `tbs=qdr:d` | Past day |
| `tbs=qdr:w` | Past week |
| `tbs=qdr:m` | Past month |
| `tbs=qdr:y` | Past year |

### DuckDuckGo Bangs Shortcuts
| Bang | Destination | Example |
|------|-------------|---------|
| `!g` | Google | `!g python tutorial` |
| `!gh` | GitHub | `!gh tensorflow` |
| `!so` | Stack Overflow | `!so python error` |
| `!w` | Wikipedia | `!w machine learning` |
| `!yt` | YouTube | `!yt tutorial` |

**Usage**: Append bang to query → `https://duckduckgo.com/html/?q=!gh+react+hooks`

## Sub-Document Index

| Document | Load Condition |
|----------|----------------|
| `advanced-google.md` | When user needs Google advanced operators, special search types, or language/region filters |
| `privacy-engines.md` | When user asks about privacy-focused search, DuckDuckGo features, or Startpage/Brave parameters |
| `wolframalpha-queries.md` | When user needs knowledge computation: math, conversions, stocks, weather, nutrition |
| `search-strategies.md` | When user needs multi-engine strategies, cross-validation, or domain-specific search patterns |

## Quick Action Patterns

### Site-Specific Search
**Pattern**: User wants results from specific website
**Action**: Use `site:` operator
**Example**: GitHub Python projects → `https://www.google.com/search?q=site:github.com+python`

### File Type Search
**Pattern**: User needs specific document format
**Action**: Use `filetype:` operator
**Example**: PDF tutorials → `https://www.google.com/search?q=tutorial+filetype:pdf`

### Privacy Search
**Pattern**: User wants untracked search
**Action**: Use DuckDuckGo or Startpage
**Example**: `https://duckduckgo.com/html/?q=privacy+tools`

### Knowledge Query
**Pattern**: User needs computation or structured data
**Action**: Use WolframAlpha
**Example**: Currency conversion → `https://www.wolframalpha.com/input?i=100+USD+to+CNY`