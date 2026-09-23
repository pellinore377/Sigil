# Reference services

Definition, weather and translation cards look things up through your server. Nothing is enabled by default. The server sends each query to the provider you choose, then encrypts the finished card when someone shares it. People receiving the card make no provider requests. The provider sees the query and your server's address, but not who asked.

## Turning providers on

1. Open `/admin` on your server and sign in.
2. Choose **Server**. It is in the left column on a wide window and the tab row on a narrow one.
3. Scroll past Registration, calls and notifications to **Reference services**.
4. Switch on the providers you want, fill in any key fields, and choose **Save reference services**.

Apps pick up the change the next time someone opens a reference tool. The same settings are available through `GET/PUT /admin/v0/services`; see `Integrations.md`.

## Providers

| Provider | Tool | Key | Cost and terms |
|---|---|---|---|
| Wiktionary | Definitions | None | Free. Content is CC BY-SA 4.0; each card credits Wiktionary. |
| Open-Meteo | Weather and place search | None | Free for non-commercial use under 10,000 calls a day. Data is CC BY 4.0; place search uses GeoNames data. |
| LibreTranslate | Translation | Optional | Free when you host it yourself. libretranslate.com requires a paid key. |
| Google Cloud Translation | Translation | Required | 500,000 characters a month free on a billing-enabled project, then billed per character. |

### Wiktionary

The preset uses the Wikimedia REST definition endpoint, `https://en.wiktionary.org/api/rest_v1/page/definition`. English Wiktionary explains words from many languages in English, and the card's language code picks the language section. Wikimedia asks automated clients to stay well below its rate limits, which the daily lookup limits handle. Its User-Agent policy also asks clients to say how to reach them, so lookups send `Sigil/experimental-v0 (+<public origin>)`, using the origin recorded when the server was claimed. A server without a public origin sends the bare agent, which Wikimedia may throttle or refuse; the composer then shows "No definition came back".

### Open-Meteo

Turning on weather adds two providers: the forecast API (`https://api.open-meteo.com/v1/forecast`) and the geocoding API (`https://geocoding-api.open-meteo.com/v1/search`) that finds the place first. The free API is for non-commercial use only. A commercial Open-Meteo subscription uses a different host and passes its key in the query string, which Sigil does not support. Keep the server-wide daily limit under 10,000 calls.

### LibreTranslate

LibreTranslate is open-source machine translation you can run yourself, for example with the official `libretranslate/libretranslate` container. Enter its address ending in `/translate`. It must be reachable over HTTPS, so put it behind your TLS reverse proxy. Your own instance needs a key only if you set one up. The hosted service at libretranslate.com requires a paid key from its portal.

Sigil does not call providers on private or loopback addresses by default. If your instance is only on your LAN, add an egress exception for its host through the API. The admin page keeps existing exceptions but cannot create them.

### Google Cloud Translation

Sigil uses the Cloud Translation Basic (v2) API with an API key.

1. In the Google Cloud console, create or choose a project and link a billing account. The monthly free allowance only applies with billing enabled.
2. Go to **APIs & Services → Library**, find **Cloud Translation API** and choose **Enable**.
3. Go to **APIs & Services → Credentials → Create credentials → API key**.
4. Edit the key. Under **API restrictions**, choose **Restrict key** and select only **Cloud Translation API**.
5. Paste the key into the admin page and save.

The first 500,000 characters each month are free, then Google bills per million characters. To avoid surprise charges, set a budget alert under **Billing → Budgets & alerts**, and cap characters per day under the API's **Quotas** page. Cards credit "Translated by Google", as Google's attribution requirements ask.

DeepL API Free (500,000 characters a month, key required) would be another option, but Sigil does not support it yet.

Do not use the unofficial `translate.googleapis.com` endpoint that some free tools call. It is not a supported API and using it breaks Google's terms.

## Keys

Keys are write-only. The admin page shows only whether a key is saved, and the field clears after saving. Leave the field empty to keep the saved key, or type a new one to replace it. Changing a LibreTranslate address removes its saved key unless you enter the key again.

## Daily limits

- **Lookups per person per day** (default 100) caps what one account can spend.
- **Lookups per day for the whole server** (default 2,000) keeps the server inside provider allowances.

Every lookup counts once against both limits, across all providers, including failed and cached lookups. Limits reset at 00:00 UTC. Definitions are cached on the server for seven days.
