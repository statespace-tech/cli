# FastAPI shop

A small FastAPI shop that runs behind `ssp host`. It prints a `signup` outcome for each
signup and a `purchase` outcome, with its `amount`, for each order. A variant is a branch
of `main.py`, such as one that lowers `PRICE`.

| File | Purpose |
| --- | --- |
| `main.py` | The app. It reads the subject from the `x-statespace-subject` header |
| `build.sh` | Builds a ready-to-run copy: the code and its dependencies in `deps/` |
| `variant.yml` | A GitHub Actions workflow that publishes each branch as a variant |
| `Dockerfile` | A production image that starts the shop with `ssp host` |
| `ssp-host.service` | The same, as a systemd service |

## Try it locally

Build control and a variant, and choose the variant per request with a header.

```shell
./build.sh /tmp/shop && cp -R /tmp/shop /tmp/cheaper
sed -i.bak 's/^PRICE = 40/PRICE = 35/' /tmp/cheaper/main.py
cd /tmp/shop && ssp host --local --health /health --variant cheaper=/tmp/cheaper \
  --command "PYTHONPATH=deps exec python3 -m uvicorn main:app --host 127.0.0.1 --port {port} --workers 2"
curl -H 'x-statespace-variant: cheaper' localhost:8080/
```

## Run an experiment

Create the app, a signing key, and a key for CI. Store the signing key and the CI key as
the repository secrets `SSP_SIGNING_KEY` and `SSP_API_KEY`, and copy `variant.yml` to
`.github/workflows/`.

```shell
ssp app create shop
ssp variant keygen
ssp key create --name ci --scope experiments:write
```

Each push to a branch now publishes it as a variant named after the branch. Start an
experiment with one.

```shell
ssp experiment create price --app shop --variant lower-price=0.5 --traffic 0.1 \
  --note "A lower price should raise revenue per visitor."
ssp experiment start price
ssp experiment results price --outcome purchase --metric amount
```

## Deploy

Run the image, or the systemd service, wherever the shop runs now. Give it a runtime key
from `ssp key create --name production --preset runtime` as `SSP_API_KEY`, and the public
signing key as `SSP_TRUST_KEY`.

```shell
docker build -t shop .
docker run -p 8080:8080 -e SSP_API_KEY -e SSP_TRUST_KEY shop
```

Variants run inside the same image, so CI builds them in `python:3.12-slim` too. When you
change the base image, change it in `variant.yml` as well.

Behind a load balancer, run as many hosts as before. Every host assigns a subject the
same variant, so requests need no stickiness. Point health checks at `/health`: control
answers them, so they pass while variants start. On deploys, the platform's `SIGTERM`
lets in-flight requests finish for up to 10 seconds; allow at least that long before it
kills the container.

Each host runs control and up to `--max-variants` variants, 4 by default. Size machines for
that many copies of the app, or lower the limit; `ssp experiment start` refuses an
experiment that hosts have no room to run.
