# Diff two profile manifests to see if there are any substantive differences.
# When the catalog moves forward but no packages actually change, the manifest
# files will have different urls but produce the same result. In this case
# we should strip the urls from the manifest data before determining if anything
# has actually changed.
#
# Invoke with:
#   jq -n -f lib/diff-manifests.jq \
#     --slurpfile m1 path/to/manifest1.json \
#     --slurpfile m2 path/to/manifest2.json

def stripUrls(manifest):
    (manifest.elements | map(del(.url))) as $elementsWithoutUrls |
    manifest * { "elements": $elementsWithoutUrls };

stripUrls($m1[0]) as $m1WithoutUrls |
stripUrls($m2[0]) as $m2WithoutUrls |
if ($m1WithoutUrls == $m2WithoutUrls) then empty else halt_error end
