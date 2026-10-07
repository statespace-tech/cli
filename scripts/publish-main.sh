#!/bin/sh
# Upload all verified native binaries before changing the main build pointer.
set -eu

commit=$1
case "$commit" in
  *[!a-f0-9]* | "") echo 'Invalid commit.' >&2; exit 1 ;;
esac
[ "${#commit}" -eq 40 ] || { echo 'Invalid commit length.' >&2; exit 1; }
bucket='statespace-frontend-production-websitebucket-hibvvyqul0t3'
distribution='E2ICCSLU66VYKY'

for target in x86_64-unknown-linux-musl aarch64-unknown-linux-musl x86_64-apple-darwin aarch64-apple-darwin; do
  asset="ssp-${target}.tar.gz"
  test -f "dist/${asset}"
  test -f "dist/${asset}.sha256"
  (cd dist && sha256sum --check "${asset}.sha256")
done
for asset in dist/*.tar.gz dist/*.sha256; do
  aws s3api put-object --bucket "$bucket" --key "cli/${commit}/${asset##*/}" \
    --body "$asset" --if-none-match '*' \
    --cache-control 'public,max-age=31536000,immutable' --query ETag --output text
done
printf '%s\n' "$commit" > dist/latest.txt
aws s3 cp dist/latest.txt "s3://${bucket}/cli/latest.txt" \
  --content-type text/plain --cache-control 'no-cache' --only-show-errors
aws cloudfront create-invalidation --distribution-id "$distribution" \
  --paths /cli/latest.txt --query 'Invalidation.Id' --output text
