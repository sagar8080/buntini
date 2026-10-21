# Public page research and price evidence

The controller owns online consent. Research reads at most three URLs from its
own selected search results. `research::fetch_selected_html` requires an exact
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
`retrievedAt`. A site requiring JavaScript, authentication, a redirect, a larger
response or an unsupported encoding can be opened in the user's browser; the
app reports its limitation rather than inventing page content.

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
