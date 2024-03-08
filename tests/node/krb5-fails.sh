set -euxo pipefail

# so npm doesn't search up
echo '{}' > package.json
if npm install krb5; then
  exit 1
fi
