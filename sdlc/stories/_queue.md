| Story | Spec | Title | Depends on | Tier | Status | Branch | Base |
|---|---|---|---|---|---|---|---|
| US-0001 | SPEC-0019 | table artifacts from text extraction | main | 1 | landed | story/US-0001-table-artifacts-from-text-extraction | main |
| US-0002 | SPEC-0019 | converter bundle and conversion capture | US-0001 | 1 | landed | story/US-0002-converter-bundle-and-conversion-capture | story/US-0001-table-artifacts-from-text-extraction |
| US-0003 | SPEC-0019 | get a table as markdown or csv | US-0002 | 1 | landed | story/US-0003-get-a-table-as-markdown-or-csv | story/US-0002-converter-bundle-and-conversion-capture |
| US-0004 | SPEC-0019 | table refusals and table routes | US-0003 | 1 | landed | story/US-0004-table-refusals-and-table-routes | story/US-0003-get-a-table-as-markdown-or-csv |
| US-0005 | SPEC-0019 | list and get images | US-0004 | 1 | landed | story/US-0005-list-and-get-images | story/US-0004-table-refusals-and-table-routes |
| US-0006 | SPEC-0019 | image failures and picture formats | US-0005 | 2 | landed | story/US-0006-image-failures-and-picture-formats | story/US-0005-list-and-get-images |
| US-0007 | SPEC-0019 | images through every conversion path | US-0006 | 1 | landed | story/US-0007-images-through-every-conversion-path | story/US-0006-image-failures-and-picture-formats |
| US-0008 | SPEC-0019 | full document view | US-0007 | 1 | landed | story/US-0008-full-document-view | story/US-0007-images-through-every-conversion-path |
| US-0009 | SPEC-0019 | view line rules and sibling independence | US-0008 | 2 | landed | story/US-0009-view-line-rules-and-sibling-independence | story/US-0008-full-document-view |
| US-0010 | SPEC-0019 | re-conversion replaces the set | US-0009 | 1 | landed | story/US-0010-re-conversion-replaces-the-set | story/US-0009-view-line-rules-and-sibling-independence |
| US-0011 | SPEC-0019 | refused and overtaken re-conversions | US-0010 | 1 | landed | story/US-0011-refused-and-overtaken-re-conversions | story/US-0010-re-conversion-replaces-the-set |
| US-0012 | SPEC-0019 | changed documents lose their artifacts | US-0011 | 1 | landed | story/US-0012-changed-documents-lose-their-artifacts | story/US-0011-refused-and-overtaken-re-conversions |
| US-0013 | SPEC-0019 | artifacts are not files, stale sets swept | US-0012 | 2 | landed | story/US-0013-artifacts-are-not-files-stale-sets-swept | story/US-0012-changed-documents-lose-their-artifacts |
| US-0014 | SPEC-0019 | consumer parity, marker scope and docs | US-0013 | 1 | landed | story/US-0014-consumer-parity-marker-scope-and-docs | story/US-0013-artifacts-are-not-files-stale-sets-swept |
| US-0015 | SPEC-0020 | project usage and quota read | main | 1 | landed | story/US-0015-project-usage-and-quota-read | main |
| US-0016 | SPEC-0020 | single file writes refused over the limit | US-0015 | 1 | landed | story/US-0016-single-file-writes-refused-over-the-limi | story/US-0015-project-usage-and-quota-read |
| US-0017 | SPEC-0020 | conversion markdown under the limit | US-0016 | 2 | landed | story/US-0017-conversion-markdown-under-the-limit | story/US-0016-single-file-writes-refused-over-the-limi |
| US-0018 | SPEC-0020 | archive and folder copy all or nothing | US-0017 | 2 | landed | story/US-0018-archive-and-folder-copy-all-or-nothing | story/US-0017-conversion-markdown-under-the-limit |
| US-0019 | SPEC-0020 | version control operations under the limit | US-0018 | 1 | landed | story/US-0019-version-control-operations-under-the-lim | story/US-0018-archive-and-folder-copy-all-or-nothing |
| US-0020 | SPEC-0020 | lowered limit, exempt operations, concurrency | US-0019 | 1 | landed | story/US-0020-lowered-limit-exempt-operations-concurre | story/US-0019-version-control-operations-under-the-lim |
| US-0021 | SPEC-0020 | remaining version control operations proven under the limit | US-0020 | 1 | landed | story/US-0021-remaining-version-control-quota-tests | story/US-0020-lowered-limit-exempt-operations-concurre |
| US-0022 | SPEC-0021 | Refused branch checkout creates no branch | main | 2 | landed | story/US-0022-refused-branch-checkout-creates-no-branch | main |
| US-0023 | SPEC-0022 | Absent values accepted on PostgreSQL | main | 1 | landed | story/US-0023-absent-values-accepted-on-postgresql | main |
| US-0024 | SPEC-0023 | SQL Server metadata store opens and lists the trash | main | 1 | landed | story/US-0024-sqlserver-metadata-store-opens | main |
| US-0025 | SPEC-0024 | Volume teardown removes every metadata row | main | 1 | landed | story/US-0025-volume-teardown-removes-every-row | main |
| US-0026 | SPEC-0024 | Project removal wipes the search index | US-0025 | 1 | landed | story/US-0026-project-removal-wipes-search-index | story/US-0025-volume-teardown-removes-every-row |
| US-0027 | SPEC-0025 | Index walks skip the trash | main | 2 | landed | story/US-0027-index-walks-skip-the-trash | main |
| US-0028 | SPEC-0025 | Auto purge and restore keep search in step | US-0027 | 2 | landed | story/US-0028-purge-and-restore-keep-search-in-step | story/US-0027-index-walks-skip-the-trash |
| US-0029 | SPEC-0026 | PostgreSQL store opens without locking a current schema | main | 1 | landed | story/US-0029-postgres-open-without-locking | main |
| US-0030 | SPEC-0027 | Disabled git and search tools are not served | main | 1 | landed | story/US-0030-disabled-git-search-not-served | main |
| US-0031 | SPEC-0027 | Enabled optional families are served over MCP | US-0030 | 1 | landed | story/US-0031-optional-families-served-over-mcp | story/US-0030-disabled-git-search-not-served |
| US-0032 | SPEC-0029 | Bump rustls past the four advisories | main | 3 | landed | story/US-0032-bump-rustls-past-advisories | main |
| US-0033 | SPEC-0030 | Clone failure test runs offline | main | 2 | landed | story/US-0033-clone-failure-test-runs-offline | main |
| US-0034 | SPEC-0031 | Pin the untested MCP protocol behaviors | main | 2 | landed | story/US-0034-pin-untested-mcp-protocol-behaviors | main |
