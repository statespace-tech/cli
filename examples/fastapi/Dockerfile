# The shop's image. The default and every variant are images built from this
# file: the default from main, each variant from a branch.
FROM python:3.12-slim

WORKDIR /app
COPY requirements.txt ./
RUN pip install --no-cache-dir --requirement requirements.txt \
    && useradd --system --no-create-home shop
COPY main.py ./
USER shop

EXPOSE 8000
CMD ["uvicorn", "main:app", "--host", "0.0.0.0", "--port", "8000", "--workers", "2"]
