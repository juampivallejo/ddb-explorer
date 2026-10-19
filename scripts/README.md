# Local DynamoDB scripts

Run these commands from the repository root.

Run the GPUI application on the host and DynamoDB Local in Docker. Docker with
the Compose plugin must be installed and its daemon running. The Nix shell
provides the AWS CLI; otherwise install AWS CLI v2 separately.

From the repository root:

```sh
nix-shell
docker compose up -d
./scripts/seed
```

The service is available at `http://127.0.0.1:8000`. Data persists in
the Docker-managed `dynamodb-data` volume. The container runs as root internally
to write this volume, including with rootless Docker. The image is pinned to the
digest verified during setup so subsequent starts use the same version.

`scripts/ddb` always supplies the local endpoint, region `us-east-1`, and dummy
credentials (`local` / `local`). No AWS account or real credentials are needed.
For the future explorer, configure these same connection values explicitly.

```sh
./scripts/ddb list-tables
./scripts/ddb describe-table --table-name Events
./scripts/ddb scan --table-name Books
./scripts/ddb get-item --table-name Books --key '{"id":{"S":"book-1"}}'
./scripts/ddb put-item --table-name Books \
  --item '{"id":{"S":"book-custom"},"title":{"S":"My sample book"}}'
```

The seed script waits for the service, creates missing tables, and writes the
checked-in fixtures. Running it again overwrites fixture items with the same
keys, preserves other items, and leaves existing table definitions unchanged.
It retries unprocessed batch writes. The fixtures include:

- `Books`: 8 items with a simple string primary key and varied attribute types,
  including numbers, booleans, nested maps/lists, sets, null, binary, and Unicode
  text. `book-6` and `book-7` each display 100 lines of formatted JSON in the item
  viewer; `book-8` displays 1,000 lines, using lists of excerpts to exercise
  scrolling through large items.
- `Events`: 160 items with partition key `pk`, sort key `sk`, and the
  `ByEventType` global secondary index. The original 40 events are joined by
  120 UX events: 115 share `pk = UX#PAGINATION`, and all 120 share
  `eventType = UX_TEST`. The last 20 have long sort keys, with the last 5 also
  having long partition keys. These exercise three 50-item pages, wrapping,
  resizing, GSI conditions, and scrolling without changing existing table schemas.
- `EmptyTable`: a table with no seeded items.
- `SampleTable001` through `SampleTable097`: empty tables with string `id`
  primary keys, bringing the seeded total to 100 tables for sidebar browsing
  and filtering. The seed script generates these definitions automatically.

Query a partition and its index:

```sh
./scripts/ddb query --table-name Events \
  --key-condition-expression 'pk = :pk' \
  --expression-attribute-values '{":pk":{"S":"USER#1"}}'

./scripts/ddb query --table-name Events --index-name ByEventType \
  --key-condition-expression 'eventType = :type' \
  --expression-attribute-values '{":type":{"S":"LOGIN"}}'
```

For pagination practice, request a small page. `--no-paginate` prevents the CLI
from fetching subsequent pages automatically; the result includes
`LastEvaluatedKey`, which can be passed as `--exclusive-start-key`.

```sh
./scripts/ddb scan --table-name Events --limit 5 --no-paginate
```

Stop the database with `docker compose down`; this preserves its data.
To deliberately erase all local tables and items and reload the fixtures:

```sh
docker compose down --volumes
docker compose up -d
./scripts/seed
```

Use `docker compose logs dynamodb` to inspect startup failures. DynamoDB Local
does not reproduce all cloud behavior or validate production IAM permissions.
The explorer should expose read operations; fixture writes stay in these tools.

## Explorer UX checks

After `./scripts/seed`, select **Events** in the app:

1. Query **Primary key** with `pk = UX#PAGINATION`, sort **Any**. Press Enter:
   expect pages of 50, 50, and 15 items.
2. Select **ByEventType**, set `eventType = UX_TEST`, and choose **Between**.
   Use `2026-10-01T00:00:00Z` through `2026-10-01T01:59:59Z`: expect 120
   items across three pages. Try **Begins with** `2026-10-01T01:` for 60 items.
3. At 800 × 600, verify the settings scroll and the Run button stays visible.
   Collapse settings with Ctrl+Shift+Q and hide the sidebar with Ctrl+B.
   Reopen both and check that values, selection, widths, and scroll are retained.
4. On the third GSI page, inspect long keys. Resize the key list using the
   divider, hover a key for its full value, and copy it with its copy button.
5. Leave a required field empty and submit: settings should expand and the
   invalid field should receive focus. Ctrl+Enter in Scan mode must not confirm
   a prepared scan.

CLI equivalents (each call fetches one page):

```sh
./scripts/ddb query --table-name Events --limit 50 --no-paginate \
  --key-condition-expression 'pk = :pk' \
  --expression-attribute-values '{":pk":{"S":"UX#PAGINATION"}}'

./scripts/ddb query --table-name Events --index-name ByEventType \
  --limit 50 --no-paginate \
  --key-condition-expression 'eventType = :type AND sk BETWEEN :start AND :end' \
  --expression-attribute-values '{":type":{"S":"UX_TEST"},":start":{"S":"2026-10-01T00:00:00Z"},":end":{"S":"2026-10-01T01:59:59Z"}}'
```
