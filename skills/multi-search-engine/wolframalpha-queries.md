# WolframAlpha Knowledge Queries

## Query Categories

### Mathematics
| Query Type | Example |
|------------|--------|
| Integration | `integrate x^2 dx` |
| Derivatives | `derivative of sin(x)` |
| Equations | `solve x^2-5x+6=0` |
| Limits | `limit of 1/x as x->infinity` |
| Matrices | `determinant of {{1,2},{3,4}}` |

### Unit Conversions
| Query Type | Example |
|------------|--------|
| Distance | `100 miles to km` |
| Temperature | `100 Fahrenheit to Celsius` |
| Weight | `50 kg to lbs` |
| Volume | `1 gallon to liters` |

### Currency & Finance
| Query Type | Example |
|------------|--------|
| Exchange rate | `100 USD to CNY` |
| Crypto | `1 BTC to USD` |
| Stock price | `AAPL stock` |
| Company data | `market cap of Tesla` |

### Science & Nature
| Query Type | Example |
|------------|--------|
| Chemistry | `molar mass of H2SO4` |
| Physics | `speed of light` |
| Elements | `properties of gold` |
| Astronomy | `distance to Mars` |

### Geography & Weather
| Query Type | Example |
|------------|--------|
| Weather | `weather in Beijing` |
| Population | `population of China` |
| GDP | `GDP of USA vs China` |
| Coordinates | `coordinates of Paris` |

### Time & Dates
| Query Type | Example |
|------------|--------|
| Date difference | `days between Jan 1 2020 and Dec 31 2024` |
| Time zone | `10am Beijing to New York` |
| Day of week | `what day was July 20 1969` |

### Nutrition & Health
| Query Type | Example |
|------------|--------|
| Calories | `calories in banana` |
| Protein | `protein in chicken breast` |
| Nutrition | `nutrition of apple` |

### Other Useful Queries
| Query Type | Example |
|------------|--------|
| IP lookup | `8.8.8.8` |
| Flight info | `flight AA123` |
| Barcode | `barcode 123456789` |
| Historical events | `events on July 20 1969` |

## WolframAlpha Query Rules

### Rule: Mathematical Computation
**Pattern**: User needs math calculation
**Action**: Construct WolframAlpha URL with URL-encoded formula
**Example**: Integral of sin(x) → `https://www.wolframalpha.com/input?i=integrate+sin%28x%29+from+0+to+pi`

### Rule: Real-Time Data
**Pattern**: User needs current stock/currency/weather data
**Action**: Use WolframAlpha for structured real-time queries
**Example**: Stock price → `https://www.wolframalpha.com/input?i=AAPL+stock+price`

### Rule: Comparison Queries
**Pattern**: User wants to compare values
**Action**: Use `vs` keyword in WolframAlpha
**Example**: GDP comparison → `https://www.wolframalpha.com/input?i=GDP+of+China+vs+USA`

### Rule: Unit Conversion
**Pattern**: User needs unit conversion
**Action**: Use `to` keyword
**Example**: `https://www.wolframalpha.com/input?i=100+miles+to+km`

## URL Encoding Notes

When constructing WolframAlpha URLs:
- Encode special characters: `(` → `%28`, `)` → `%29`, `^` → `%5E`
- Replace spaces with `+`
- Example: `x^2` → `x%5E2`