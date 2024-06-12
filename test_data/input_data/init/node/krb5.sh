set -euxo pipefail

# so npm doesn't search up
echo '{}' > package.json
npm install krb5
[ -x ./node_modules/krb5/build/Release/krb5.node ]
