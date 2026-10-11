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

- `Books`: a simple string primary key and varied attribute types, including
  numbers, booleans, nested maps/lists, sets, null, binary, and Unicode text.
- `Events`: 40 items with partition key `pk`, sort key `sk`, and the
  `ByEventType` global secondary index.
- `EmptyTable`: a table with no seeded items.

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
