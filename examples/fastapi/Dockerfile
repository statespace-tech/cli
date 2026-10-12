# The shop's production image. `ssp host` replaces the start command: it runs
# this code as control, and the variants of running experiments beside it.
FROM python:3.12-slim

RUN apt-get update \
    && apt-get install --yes --no-install-recommends curl ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && curl -fsSL https://statespace.com/install \
       | STATESPACE_VERSION=0.2.0 STATESPACE_INSTALL_DIR=/usr/local/bin sh

WORKDIR /app
COPY build.sh requirements.txt main.py ./
RUN ./build.sh /app/build && useradd --system --create-home shop \
    && mkdir -p /var/cache/statespace && chown shop /var/cache/statespace
WORKDIR /app/build
USER shop

# SSP_API_KEY (a runtime key) and SSP_TRUST_KEY come from the platform's secrets.
ENV STATESPACE_APP=shop STATESPACE_CACHE=/var/cache/statespace STATESPACE_LISTEN=0.0.0.0:8080
EXPOSE 8080
CMD exec ssp host --health /health --trust-key "$SSP_TRUST_KEY" \
    --command "PYTHONPATH=deps exec python3 -m uvicorn main:app --host 127.0.0.1 --port {port} --workers 2" \
    --capture stderr --redact 'sk_live_\w+'
