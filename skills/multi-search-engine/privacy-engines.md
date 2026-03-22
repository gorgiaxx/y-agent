# Privacy-Focused Search Engines

## DuckDuckGo Features

### Built-in Tools (No External Site Needed)
| Query | Function | Example |
|-------|----------|--------|
| `password {n}` | Generate n-character password | `password 20` |
| `base64 {text}` | Base64 encode | `base64 hello world` |
| `#{color}` | Color code info | `#FF5733` |
| `qr {text}` | Generate QR code | `qr https://example.com` |
| `uuid` | Generate UUID | `uuid` |
| `shorten {url}` | Create short link | `shorten example.com` |

### DuckDuckGo Parameters
| Parameter | Purpose | Example |
|-----------|---------|--------|
| `kp=1` | Strict safe search | `&kp=1` |
| `kp=-1` | Disable safe search | `&kp=-1` |
| `kl=cn` | China region | `&kl=cn` |
| `kl=us-en` | US English | `&kl=us-en` |
| `ia=images` | Image results | `&ia=images` |
| `ia=news` | News results | `&ia=news` |
| `ia=videos` | Video results | `&ia=videos` |

### Extended Bangs Reference

**Development**:
- `!npm` → npmjs.com
- `!pypi` → PyPI
- `!mdn` → MDN Web Docs
- `!docker` → Docker Hub

**Knowledge**:
- `!wen` → Wikipedia English
- `!wt` → Wiktionary
- `!imdb` → IMDb

**Shopping**:
- `!a` → Amazon
- `!e` → eBay
- `!ali` → AliExpress

**Maps**:
- `!m` → Google Maps
- `!maps` → OpenStreetMap

## Startpage Features

| Feature | Description |
|---------|-------------|
| Anonymous View | Click "Anonymous View" to browse results via proxy |
| EU Servers | Data protected under EU privacy laws |
| No Tracking | No search history recorded |

### Startpage Parameters
| Parameter | Purpose |
|-----------|---------|
| `cat=images` | Image search |
| `cat=video` | Video search |
| `cat=news` | News search |
| `time=day` | Past 24 hours |
| `time=week` | Past week |
| `time=month` | Past month |
| `language=english` | English results |

## Brave Search Features

| Feature | Description |
|---------|-------------|
| Independent Index | Own crawler, not dependent on Google/Bing |
| Goggles | Custom search filters |
| Discussions | Aggregates forum discussions (Reddit, etc.) |

### Brave Parameters
| Parameter | Purpose |
|-----------|---------|
| `tf=pw` | Past week |
| `tf=pm` | Past month |
| `tf=py` | Past year |
| `source=news` | News search |
| `source=images` | Image search |
| `source=videos` | Video search |

## Privacy Comparison

| Engine | Tracking | Data Retention | Best For |
|--------|----------|----------------|----------|
| DuckDuckGo | None | None | Daily privacy search |
| Startpage | None | None | Google results + privacy |
| Brave | None | None | Independent index |
| Qwant | None | None | EU GDPR compliance |

## Privacy Search Rules

### Rule: Sensitive Topic Search
**Pattern**: User searches for sensitive topics
**Action**: Use DuckDuckGo or Startpage, never Google/Bing
**Example**: `https://duckduckgo.com/html/?q=health+condition`

### Rule: Anonymous Browsing
**Pattern**: User wants to visit result without revealing IP
**Action**: Use Startpage's "Anonymous View" feature

### Rule: No Filter Bubble
**Pattern**: User wants unbiased, unpersonalized results
**Action**: Use Brave or DuckDuckGo (no personalization algorithms)