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
| US-0035 | SPEC-0032 | Doc service timeout test proves the kill | main | 2 | landed | story/US-0035-doc-service-timeout-test-proves-kill | main |
| US-0036 | SPEC-0033 | A refused artifact commit releases its blobs | main | 1 | landed | story/US-0036-refused-artifact-commit-releases-blobs | main |
| US-0037 | SPEC-0034 | The blob store lists its objects and refreshes re stored blobs | main | 2 | landed | story/US-0037-blob-store-lists-and-refreshes | main |
| US-0038 | SPEC-0034 | Sweep the unreferenced blobs of one project | US-0037 | 1 | landed | story/US-0038-orphan-sweep-for-one-project | story/US-0037-blob-store-lists-and-refreshes |
| US-0039 | SPEC-0034 | The purge cycle runs the orphan sweep once a day | US-0038 | 2 | landed | story/US-0039-purge-cycle-runs-orphan-sweep-daily | story/US-0038-orphan-sweep-for-one-project |
| US-0040 | SPEC-0035 | git2 0.21 and yoke-derive 0.8.4 with unchanged behavior | main | 2 | landed | story/US-0040-git2-021-and-yoke-derive-084 | main |
| US-0041 | SPEC-0035 | Record the accepted advisories and drop the stale ignores | US-0040 | 2 | landed | story/US-0041-record-accepted-advisories | story/US-0040-git2-021-and-yoke-derive-084 |
| US-0042 | SPEC-0036 | zip 9 with declared decoding, dedup and writer changes | main | 1 | landed | story/US-0042-zip-9-upgrade | main |
| US-0043 | SPEC-0036 | narrow the zip acceptances and drop stale citations | US-0042 | 2 | landed | story/US-0043-narrow-zip-acceptances | story/US-0042-zip-9-upgrade |
| US-0044 | SPEC-0037 | pin the docx envelope and the zip size message | main | 2 | landed | story/US-0044-pin-docx-envelope-and-zip-message | main |
| US-0045 | SPEC-0038 | local put rewrites a blob deleted mid call | main | 2 | landed | story/US-0045-local-put-self-heal | main |
| US-0046 | SPEC-0039 | BT-020 waits for indexing before its baseline | main | 2 | landed | story/US-0046-bt020-deterministic-baseline | main |
| US-0047 | SPEC-0040 | replace serde_yaml with serde_norway | main | 2 | landed | story/US-0047-replace-serde-yaml | main |
| US-0048 | SPEC-0041 | refuse inconsistent zips on archive extraction | main | 1 | landed | story/US-0048-refuse-inconsistent-zip-extraction | main |
| US-0049 | SPEC-0041 | refuse inconsistent documents on text extraction and conversion | US-0048 | 1 | landed | story/US-0049-refuse-inconsistent-documents | story/US-0048-refuse-inconsistent-zip-extraction |
| US-0050 | SPEC-0042 | move the toolchain to 1.94.1 and lock aws-sdk-s3 1.152.0 | main | 2 | landed | story/US-0050-rust-toolchain-1-94 | main |
| US-0051 | SPEC-0043 | send the Accept header on the full-stack e2e MCP calls | main | 2 | landed | story/US-0051-full-stack-e2e-accept-header | main |
| US-0052 | SPEC-0044 | add the joint rename and trash entry operation | main | 2 | landed | story/US-0052-joint-rename-trash-entry | main |
| US-0053 | SPEC-0044 | route soft delete and purge through the joint operation | US-0052 | 2 | landed | story/US-0052-joint-rename-trash-entry | main |
| US-0054 | SPEC-0044 | seal the old write path and document it | US-0053 | 2 | landed | story/US-0053-route-delete-and-purge-through-joint-op | story/US-0052-joint-rename-trash-entry |
| US-0055 | SPEC-0045 | compare two text files with fs.diff_files | main | 1 | landed | story/US-0055-compare-two-text-files-fs-diff-files | main |
| US-0056 | SPEC-0045 | refuse what cannot be compared | US-0055 | 2 | landed | story/US-0056-refuse-what-cannot-be-compared | story/US-0055-compare-two-text-files-fs-diff-files |
| US-0057 | SPEC-0045 | text only and a size limit | US-0056 | 2 | landed | story/US-0057-text-only-and-size-limit | story/US-0056-refuse-what-cannot-be-compared |
| US-0058 | SPEC-0045 | a comparison is read only and counts as access | US-0055 | 2 | landed | story/US-0058-diff-is-read-only-and-counts-as-access | story/US-0057-text-only-and-size-limit |
| US-0059 | SPEC-0045 | the same comparison on the REST data plane | US-0056 | 2 | landed | story/US-0059-rest-route-get-api-fs-diff | story/US-0058-diff-is-read-only-and-counts-as-access |
| US-0060 | SPEC-0045 | large comparison bound, docs, and spec close | US-0057, US-0058, US-0059 | 2 | landed | story/US-0060-diff-performance-bound-docs-and-spec-close | story/US-0059-rest-route-get-api-fs-diff |
| US-0061 | SPEC-0046 | choose the context size of a comparison | main | 2 | landed | story/US-0061-choose-context-lines-on-the-tool | main |
| US-0062 | SPEC-0046 | the same context size on the web interface | US-0061 | 2 | landed | story/US-0062-context-lines-on-the-web-interface | story/US-0061-choose-context-lines-on-the-tool |
| US-0063 | SPEC-0046 | other diffs unchanged, scale, docs, spec close | US-0061, US-0062 | 2 | landed | story/US-0063-other-diffs-unchanged-scale-docs-and-spec-close | story/US-0062-context-lines-on-the-web-interface |
| US-0064 | SPEC-0047 | list a project's largest files | main | 1 | landed | story/US-0064-list-a-projects-largest-files | main |
| US-0065 | SPEC-0047 | show trashed files and the shared population | US-0064 | 2 | landed | story/US-0065-show-trashed-files-and-shared-population | story/US-0064-list-a-projects-largest-files |
| US-0066 | SPEC-0047 | the same breakdown on the web interface | US-0064 | 2 | landed | story/US-0066-the-same-breakdown-on-the-web-interface | story/US-0065-show-trashed-files-and-shared-population |
| US-0067 | SPEC-0047 | one moment, read only, hardening, docs, spec close | US-0065, US-0066 | 1 | landed | story/US-0067-one-moment-read-only-hardening-and-spec-close | story/US-0066-the-same-breakdown-on-the-web-interface |
