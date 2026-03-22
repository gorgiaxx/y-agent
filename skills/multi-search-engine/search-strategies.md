# Multi-Engine Search Strategies

## Engine Selection by Intent

| Search Goal | Primary Engine | Alternative | Reason |
|-------------|----------------|-------------|--------|
| Academic research | Google Scholar | Brave | Academic index |
| Programming | Google | DuckDuckGo (!gh, !so) | Technical docs |
| Privacy-sensitive | DuckDuckGo | Startpage, Brave | No tracking |
| Real-time news | Brave News | Google News | Independent index |
| Knowledge computation | WolframAlpha | Google | Structured data |
| Chinese content | Google HK | Bing CN | Chinese optimization |
| European perspective | Qwant | Startpage | EU compliance |
| Eco-friendly | Ecosia | DuckDuckGo | Plants trees |
| Unfiltered results | Brave | Startpage | No bias |

## Cross-Engine Validation Strategy

### Rule: Verify Important Information
**Pattern**: User needs reliable, verified information
**Action**: Search same keyword across multiple engines and compare
**Example Process**:
1. Google: `https://www.google.com/search?q={keyword}&tbs=qdr:m`
2. Brave: `https://search.brave.com/search?q={keyword}&tf=pm`
3. DuckDuckGo: `https://duckduckgo.com/html/?q={keyword}`
4. Compare results for consistency

## Time-Sensitive Search

| Urgency | Engine | Parameter |
|---------|--------|----------|
| Real-time (hours) | Google News, Brave News | `tbs=qdr:h`, `source=news` |
| Recent (days) | Google, Brave | `tbs=qdr:d`, `time=day` |
| This week | All engines | `tbs=qdr:w`, `tf=pw` |
| This month | All engines | `tbs=qdr:m`, `tf=pm` |
| Historical | Google Scholar | Academic archives |

## Domain-Specific Search Patterns

### Technical Development
**Pattern**: User searches for code/technical content
**Action**: Use DuckDuckGo Bangs for direct access
- GitHub projects: `!gh {query}`
- Stack Overflow: `!so {query}`
- MDN docs: `!mdn {query}`
- npm packages: `!npm {query}`
- PyPI packages: `!pypi {query}`

### Academic Research
**Pattern**: User needs scholarly sources
**Action**: 
1. Google Scholar for papers
2. `filetype:pdf` for PDF documents
3. `site:arxiv.org` for preprints

### Financial Research
**Pattern**: User needs financial data
**Action**:
1. WolframAlpha for real-time stocks/currency
2. `filetype:pdf` for earnings reports
3. `site:sec.gov` for regulatory filings

### News & Current Events
**Pattern**: User needs breaking news
**Action**:
1. Google News with hour filter: `&tbm=nws&tbs=qdr:h`
2. Brave News: `&source=news`
3. DuckDuckGo news: `&ia=news`

## URL Construction Rules

### Rule: URL Encoding
**Pattern**: Keyword contains spaces or special characters
**Action**: 
- Replace spaces with `+` or `%20`
- Encode special characters: `"` → `%22`, `&` → `%26`, `#` → `%23`

### Rule: Parameter Combination
**Pattern**: Multiple filters needed
**Action**: Combine parameters with `&`
**Example**: News from past week in Chinese → `https://www.google.com/search?q={keyword}&tbm=nws&tbs=qdr:w&lr=lang_zh-CN`

## Best Practices Summary

1. **Start broad, then narrow**: Begin with general search, add operators progressively
2. **Use time filters**: Essential for news, tech trends, and rapidly evolving topics
3. **Cross-validate**: Important claims should be verified across multiple engines
4. **Respect privacy**: Use privacy engines for sensitive queries
5. **Leverage Bangs**: DuckDuckGo Bangs provide direct access to specialized sites
6. **Know your engine**: Each engine has strengths—match engine to task