# The tiny app as an image. Build a variant with another greeting:
#   docker build --build-arg GREETING=hi -t app:hi examples/app
FROM python:3.12-alpine
ARG GREETING=hello
ENV GREETING=$GREETING
COPY app.py /app/app.py
EXPOSE 8000
CMD ["python3", "/app/app.py"]
