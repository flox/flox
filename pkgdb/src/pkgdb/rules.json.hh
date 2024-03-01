/*  Rules file for scraping.
    Four entries are allowed, each containing an array of attribute paths, in
   array form. `null` is allowed only in the `system` level of the attribute
   path and will be replaced with four rules, one for each of the default
   systems.

    The entries are `allowRecursive`, `disallowRecursive`, `allowPackage`,
   `diallowPackage`.  Note that currently a `disallowRecursive` will override
   nested `allow*` rules.  This is not currently needed but could be allowed in
   the future if needed.

   Current rules explanation:
    - legacyPackages.*.darwin is not scraped using default logic
    - legacyPackages.*.swiftPackages.darwin is not scraped using default logic
*/
R"_JSON(
{
    "allowRecursive": [
       ["legacyPackages", null, "darwin"],
       ["legacyPackages", null, "swiftPackages", "darwin"]
    ]
}
)_JSON"
