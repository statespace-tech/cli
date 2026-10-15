"""A small shop for trying Statespace with FastAPI.

Run it behind `ssp proxy`. It reads the subject from the `x-statespace-subject`
header and prints a JSON line for each signup and purchase; Statespace records
every line the arms print. A variant is a branch of this file, built into an
image and signed in CI.
"""

import json
import logging
import os

from fastapi import FastAPI, Header, HTTPException
from pydantic import BaseModel

# The change a variant tests. A branch might lower the price or rewrite checkout.
PRICE = 40

logging.basicConfig(level=logging.INFO, format="%(levelname)s %(message)s")
log = logging.getLogger("shop")
app = FastAPI()


def event(name: str, subject: str | None, **data: object) -> None:
    """Print one JSON line. Its `subject` ties it to the subject's arm in SQL."""
    print(json.dumps({"event": name, "subject": subject, **data}), flush=True)


class Signup(BaseModel):
    email: str


class Order(BaseModel):
    quantity: int = 1


@app.get("/health")
def health() -> dict[str, str]:
    return {"status": "ok"}


@app.get("/")
def home() -> dict[str, object]:
    return {"variant": os.environ.get("STATESPACE_VARIANT", "default"), "price": PRICE}


@app.post("/signup")
def signup(body: Signup, subject: str | None = Header(None, alias="x-statespace-subject")):
    log.info("signup for %s", body.email)
    event("signup", subject)
    return {"welcome": body.email}


@app.post("/checkout")
def checkout(body: Order, subject: str | None = Header(None, alias="x-statespace-subject")):
    if body.quantity < 1:
        raise HTTPException(422, "quantity must be at least 1")
    amount = PRICE * body.quantity
    # Logs can hold secrets. `ssp proxy --redact` removes them before they leave the machine.
    log.info("charged %s with key %s", amount, os.environ.get("PAYMENT_KEY", "sk_test_local"))
    event("purchase", subject, amount=amount, quantity=body.quantity)
    return {"amount": amount}
