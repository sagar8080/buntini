# Online integrations

Online mode can reach a small, fixed set of read-only services. Foreground web
research has one controller-owned route: browser search, selected source reads,
then a cited answer; the configured Brave or Tavily API is a fallback. The model
does not choose between those search transports on each turn.

The flow is:

1. **Plan (no network).** Direct research requests enter the web-research route.
   The saved intent retains the complete goal and exact query, including quoted
   phrases, case, exclusions and operators. An explicitly named site such as
   YouTube becomes visible `site:youtube.com` scope before approval; both
   transports then use that same approved query. Conservative literal labels retain
   comparison, freshness, source-quality and answer-format requirements. Explicit
   requests to open a search-results page remain navigation requests. For other
   online requests, `plan_online_request` may ask the model to propose a specific
   read-only integration such as weather or currency from
   `src-tauri/src/integrations/mod.rs`. Keyed integrations are offered only when
   configured. Invalid proposals fall back to a web-research plan.
2. **Validate.** Rust parses every argument against a strict per-tool schema
   (unknown keys rejected, lengths and ranges bounded, enums and ISO/IATA codes
   checked, dates limited to the next 360 days) and fills defaults so the
   approval shows exactly what will be sent. The Python worker independently
   validates the same schemas before any tool call reaches Rust.
3. **Approve.** The user sees the service, host and every outgoing value. Web
   research discloses `www.google.com`, the configured API fallback, the query,
   and up to six selected public result-page reads in a dedicated tab in your
   existing browser. Its normal profile may use existing cookies and sign-ins.
   Browser success does not need an API key. The
   approval is one-use, expires after five minutes, and is re-validated when
   consumed; a changed or widened call is refused.
4. **Execute.** Research first opens Google in a dedicated tab in an allowed
   existing/default browser and extracts
   verified result links. The controller ranks and deduplicates public HTTPS
   results, then reads at most three selected pages per route, six overall. If
   browser discovery or page evidence is insufficient, it tries the configured
   Brave or Tavily API using the same approved query. A synthesis coverage gap
   can also trigger that fallback once, preserving pages already read. Cancellation stops the
   operation without starting fallback. Specific API integrations retain their
   fixed HTTPS hosts, bounded responses and at most ten attributed cards.
5. **Summarize.** Research synthesis receives the original intent, numbered
   sources and read page text as untrusted data. It must return a complete or
   partial coverage assessment and list missing requirements. The controller
   can format citations from typed claims with explicit source numbers. It
   rejects incomplete, malformed or uncited summaries and references to pages
   that were not read. Comparisons require read evidence and citations from at
   least two distinct hosts, or two distinct pages when the query explicitly
   restricts research to one site. A malformed or uncited summary gets one bounded
   correction attempt using the same evidence; no unverified draft is streamed.
   Without a usable model summary, retrieved sources
   remain saved and research stays incomplete. Specific integrations may still
   display their original result cards without a model.

| Tool | Service and host | Key | Sends |
| --- | --- | --- | --- |
| `web_search` | Google in your existing browser; configured Brave or Tavily API fallback | Only for API fallback | The approved exact query; reads selected public result URLs |
| `weather` | Open-Meteo (`geocoding-api.open-meteo.com`, `api.open-meteo.com`) | No | Place name and units (always the 7-day forecast) |
| `currency` | Frankfurter / ECB (`api.frankfurter.dev`) | No | Amount and ISO currency codes |
| `wikipedia` | Wikipedia (`<lang>.wikipedia.org`) | No | Search text and language |
| `places` | OpenStreetMap Nominatim (`nominatim.openstreetmap.org`) | No | Location text, one of 19 categories, radius (100–5000 m) |
| `ebay_search` | eBay Browse API (`api.ebay.com`, sandbox for SBX keys) | App ID + secret | Search text, max price, condition, sort, marketplace |
| `flight_search` | Duffel (`api.duffel.com`) | Access token | Origin, destination, dates, adults, cabin |

Nominatim calls are spaced at least 1.1 seconds apart to respect its usage
policy. eBay application tokens are cached in memory until shortly before they
expire. Duffel searches create an offer request and read back the eight cheapest
offers; Buntini has no booking, payment or order endpoints. Keyed requests are
reserved against the monthly request cap before dispatch.

Foreground browser search does not consume a search-API reservation. API fallback
uses the existing provider-specific 15-minute result cache and reserves each
network attempt, including a bounded retry, against persisted count and spending
limits. Cached results use no additional reservation. Changing the configured
fallback provider invalidates a pending web approval. Recurring saved searches
continue to use their explicitly approved provider and budget; they do not open
foreground browser windows.

`cargo test live_keyless -- --ignored --nocapture` performs a real weather,
currency, Wikipedia and places lookup. eBay and Duffel parsing is covered by
fixtures; live checks need the user's own keys.

# Public page research and price evidence

The controller owns online consent. Foreground research reads at most six URLs
from its own selected search results. It creates one research tab in an allowed
existing/default browser and reuses that tab for selected sources. It does not
read other tabs' titles, addresses or page contents, or read cookie files. The browser's normal profile may supply
cookies and existing sign-ins, and page visits may enter its history. This route
does not use Buntini's separate embedded browser. A rendered read must match the
selected URL and return bounded visible text. When it cannot provide usable
content, the same selected URL may be read through the credential-free HTML
fetcher.

Native research-tab reading currently supports Chrome, Brave and Edge. Safari,
Arc and other default browsers can open Google, then use the approved API and
public HTML fallback because a stable normal-window research tab cannot be
verified through this adapter. A frontmost recognized browser takes precedence
over the macOS default. If a supported browser has no normal window, Buntini can
open one for its research tab; it does not use private windows. The research tab
remains visible for review.

Approval permits the browser navigation. Reading additionally requires allowing
the browser in Settings → Apps and granting macOS Automation permission. Page
text is read through JavaScript from Apple Events when the browser permits it,
or through existing Accessibility permission when scripting is unavailable.
Buntini does not change those browser or macOS permissions.
Browser-derived result titles, snippets and page text require Browser sharing
before they can reach a cloud brain; local brains keep that processing on the
Mac. Without the required permissions or browser support, the approved API and
public HTML fallback may still supply evidence. Saved browser-derived answers
retain Browser provenance for later conversation and task-resume checks.

`research::fetch_selected_html` requires an exact
match against that trusted result list before it performs DNS or HTTP work.
Model-generated URLs cannot expand the list. Read a selected page once, then use
`research::extract_content(page.clone())` and
`pricing::extract_offers(&page.html, &page.url)` for text and price evidence.
`research::fetch_selected` is the text-only convenience wrapper.

A saved price watch is separate authorization for repeated reads of its specific
user-entered URL. Only that controller path may call
`research::fetch_public_html`. Saving a syntactically valid URL does not establish
future network trust: public DNS validation runs again on every scheduled fetch.
Pausing/deleting a watch is enforced by the scheduler before dispatch.

## Retrieval boundaries

Research selects public HTTPS destinations and reads only its own tab. If the
user closes, moves, deselects or navigates that tab, browser automation stops;
remaining evidence may come from the approved API and public HTML fallback. The
existing browser controls page requests, redirects, subresources, cookies and
downloads under its own settings; Buntini's embedded-browser network isolation
does not apply to this route. No website form filling, purchasing, booking or
sending is performed. Opening a search-results tab is not source evidence and
cannot complete research.

The fallback HTML reader has these additional boundaries:

- Only HTTPS on port 443 is supported. Embedded credentials, local/special-use
  names, private IPs, reserved addresses, mapped IPv4 and transition IPv6
  addresses are rejected.
- All DNS answers must be public. The client pins those checked addresses,
  preserves TLS host/certificate validation, and verifies the connected peer.
  Automatic proxies and redirects are disabled. A redirect requires selecting
  its final HTTPS address explicitly.
- There are no cookies, API keys, login credentials, browser sessions, referrers,
  page scripts, subresource requests, outgoing page-link requests, or remote
  JSON-LD context requests. This is a read-only HTML GET.
- The overall operation has a 15-second deadline. DNS is limited to three
  seconds and four concurrent system lookups, with the lookup permit retained
  even when a timed-out blocking resolver finishes later.
- Response bodies are capped at 1 MiB. Only declared `text/html` or
  `application/xhtml+xml` is accepted. Unsupported compression/character
  encodings fail with an actionable error instead of being treated as text.
- Text extraction prefers `main`, `article`, a main landmark, or the body. It
  removes executable markup, forms, navigation, hidden/inert content, and
  common non-content elements. Output is plain text, capped at 12,000 Unicode
  characters and 100,000 visited DOM nodes, with an explicit `truncated` flag.

Extracted text is untrusted source material. It must remain quoted tool data,
never instructions, and any summary should cite the supplied `url` and
`retrievedAt`. A site requiring authentication, unsupported navigation, a larger
response or an unsupported encoding can remain unreadable. Browser rendering
may handle JavaScript, but neither rendering nor a successful HTTP request proves
that a source answers the user's question. The app reports missing evidence
rather than completing from search snippets alone.

## Published price extraction

`pricing::extract_offers` reads only Schema.org `application/ld+json` blocks. It
supports Product `offers` objects/arrays, standalone Offer/AggregateOffer,
`@graph` references, expanded Schema.org type names, and explicit price
specifications. It does not infer prices from arbitrary text, currency symbols,
discount badges, review scores, model numbers, or generated summaries.

`PriceOffer.price` and `highPrice` are canonical decimal strings. Supported
amounts are positive ASCII decimals with at most 18 integer and six fractional
digits. Currency must be a recognized three-letter uppercase ISO code; no
exchange-rate conversion is performed. The parser rejects malformed amounts,
missing/unknown currency, inverted aggregate ranges, non-sale business
functions, and conflicting explicit prices or currencies.

An AggregateOffer uses its published `lowPrice` as `price`, its optional
`highPrice` as `highPrice`, and sets `isAggregate=true`. This is a published range,
not a guaranteed quote for a particular merchant/variant. Underlying offers are
not duplicated when their aggregate range is returned. Offers from different
products/currencies may coexist: the controller must require an unambiguous
product/currency match for a watch instead of silently selecting a low price.

Every offer's `url` is the evidence page that was read. JSON-LD checkout or seller
URLs are never fetched or used as an action. Availability/seller information is
reported only when explicitly published. Taxes, shipping, membership pricing,
regional pricing, and current checkout totals cannot be established from this
metadata alone. Price history should store the page retrieval time alongside
the extracted decimal/currency and aggregate marker.

Parsing is bounded to 32 JSON-LD scripts, 256 KiB per script, 4,096 JSON nodes,
and 40 valid offers. Malformed JSON-LD blocks are ignored; if no supported price
remains, the operation reports that no supported structured price was found.

## Validation and references

Module tests cover private/encoded/mapped IP rejection, mixed public/private DNS
answers, exact source allowlisting, markup/hidden-content removal and Unicode
truncation, local HTTP response fixtures for redirects/MIME/encoding/body bounds,
Product arrays, aggregate ranges, graph references, malformed data, and rejection
of arbitrary page numbers or conflicting prices. The HTTP fixture bypass is
confined to tests of the response reader; public fetch APIs reject loopback HTTP.

Primary references: [Reqwest client configuration](https://docs.rs/reqwest/0.12.28/reqwest/struct.ClientBuilder.html),
[Schema.org Product](https://schema.org/Product),
[Offer](https://schema.org/Offer), and
[AggregateOffer](https://schema.org/AggregateOffer).
