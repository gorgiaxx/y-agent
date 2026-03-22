# Google Advanced Search Techniques

## Advanced Operators

| Operator | Function | Example |
|----------|----------|--------|
| `inurl:` | URL contains term | `inurl:login admin` |
| `intitle:` | Title contains term | `intitle:"index of" mp3` |
| `intext:` | Body contains term | `intext:password filetype:txt` |
| `cache:` | View cached version | `cache:example.com` |
| `related:` | Find related sites | `related:github.com` |
| `info:` | Site information | `info:example.com` |
| `*` | Wildcard | `machine * algorithms` |
| `()` | Grouping | `(apple OR microsoft) phones` |
| `..` | Number range | `laptop $500..$1000` |

## Special Search Types

| Type | URL Pattern | Example |
|------|-------------|--------|
| Images | `&tbm=isch` | `https://www.google.com/search?q=cat&tbm=isch` |
| News | `&tbm=nws` | `https://www.google.com/search?q=tech&tbm=nws` |
| Videos | `&tbm=vid` | `https://www.google.com/search?q=tutorial&tbm=vid` |
| Shopping | `&tbm=shop` | `https://www.google.com/search?q=laptop&tbm=shop` |
| Books | `&tbm=bks` | `https://www.google.com/search?q=history&tbm=bks` |
| Scholar | `https://scholar.google.com/scholar?q={keyword}` | Academic papers |

## Language and Region Filters

| Parameter | Purpose | Example |
|-----------|---------|--------|
| `hl=en` | Interface language | `&hl=en` for English UI |
| `lr=lang_zh-CN` | Result language | `&lr=lang_zh-CN` for Chinese results |
| `cr=countryCN` | Country restriction | `&cr=countryCN` for China |
| `gl=us` | Geographic location | `&gl=us` for US results |

## Custom Date Range

**Pattern**: `tbs=cdr:1,cd_min:MM/DD/YYYY,cd_max:MM/DD/YYYY`

**Example**: Search 2024 content → `&tbs=cdr:1,cd_min:1/1/2024,cd_max:12/31/2024`

## Combined Advanced Search Rules

### Rule: Academic Paper Search
**Pattern**: User needs scholarly articles
**Action**: 
1. Use Google Scholar or `filetype:pdf` operator
2. Add year filter for recent papers
**Example**: `https://scholar.google.com/scholar?q=deep+learning+2024`

### Rule: Technical Documentation
**Pattern**: User needs official docs or tutorials
**Action**: Use `site:` operator with official domains
**Example**: Python docs → `https://www.google.com/search?q=site:docs.python.org+async`

### Rule: Exclude Low-Quality Sources
**Pattern**: User wants authoritative results
**Action**: Use `-` operator to exclude sites
**Example**: `python tutorial -wikipedia -w3schools`

### Rule: Price Range Search
**Pattern**: User wants products in budget
**Action**: Use `..` range operator
**Example**: `laptop $800..$1200 reviews`