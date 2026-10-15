# FastAPI shop

A small FastAPI shop that runs behind `ssp proxy`. It prints a JSON line for each
signup and purchase, with the subject and the amount. A variant is a branch of
`main.py`, such as one that lowers `PRICE`, built into an image.

| File | Purpose |
| --- | --- |
| `main.py` | The app. It reads the subject from the `x-statespace-subject` header |
| `Dockerfile` | The shop's image: the default from `main`, each variant from a branch |
| `variant.yml` | A GitHub Actions workflow that publishes each branch's image as a variant |
| `ssp-proxy.service` | `ssp proxy` under systemd, on a machine with Docker |

For Kubernetes, see `../kubernetes`.

## Run an experiment

Create the app, a signing key, and a token for CI. Store the signing key and the
CI token as the repository secrets `SSP_SIGNING_KEY` and `SSP_TOKEN`, and copy
`variant.yml` to `.github/workflows/`.

```shell
ssp app create shop
ssp keygen
ssp token create --name ci --preset publisher
```

Each push to a branch now builds its image, pushes it to GitHub's registry, and
publishes it as a variant named after the branch. Start an experiment with one:
5% of the shop's subjects get the lower price and 5% the control arm.

```shell
ssp experiment create price --app shop --split lower-price=0.05 \
  --note "A lower price should raise revenue per visitor."
ssp experiment start price
```

Compare revenue per subject in SQL, with a token from `ssp token create --name
sql --preset analyst --app shop`:

```sql
SELECT variant, count(DISTINCT subject_id) AS subjects,
       sum((data->>'amount')::numeric) / count(DISTINCT subject_id) AS revenue_per_subject
FROM shop.logs
WHERE experiment = 'price' AND reason = 'assigned'
GROUP BY variant;
```

`shop.logs` has one row per subject's run with each line the subject's arm
printed after it, so subjects without a purchase count with no revenue.

## Deploy

Install `ssp-proxy.service` wherever the shop runs now, with Docker. Give it a
runtime token from `ssp token create --name production --preset runtime` as
`SSP_TOKEN`, and the public signing key as `SSP_TRUST_KEY`. The proxy runs the
image in `/etc/shop/image` as the default, and each arm of a running experiment
beside it. The containers inherit the service's environment, such as
`PAYMENT_KEY` from `/etc/shop/env`.

To deploy a new default, write its image to `/etc/shop/image` and run `systemctl
reload shop`. The proxy starts the new image, sends it traffic once `/health`
answers, and then stops the old container.

Behind a load balancer, run as many machines as before. Every proxy assigns a
subject the same arm, so requests need no stickiness. Each proxy runs the
default and up to `--max-variants` arms, 4 by default, control arms included.
Size machines for that many copies of the shop, or lower the limit; `ssp
experiment start` warns when an experiment needs more arms than proxies run.
