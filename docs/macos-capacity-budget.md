# macOS capacity query budget

On macOS 27, CoreFoundation's
`kCFURLVolumeAvailableCapacityForImportantUsageKey` reaches
`CacheDeleteCopyAvailableSpaceForVolume`. One query produced roughly 120
unified log lines in the reported trace. PwrDisk calls the public URL
resource API through a single probe in `macos/capacity.rs`.

| Operation | Probe calls per mounted volume | Projected log lines |
| --- | ---: | ---: |
| Startup in the source chooser | 0 | 0 |
| Startup with a local root, scan or rescan | At most 1, when the scan lands | ~120 at the call limit |
| Live meter refresh | At most 1 across all samples in 60 seconds | ~120 per minute at the call limit |
| Scan report snapshots, MCP readings and removal measurements | 0 | 0 |
| Regular test suite | 0 | 0 |
| Opt-in native integration test | 1 | ~120 |

The scan and meter share a per-process cache keyed by mount point and
source. A successful or unavailable estimate consumes the same 60-second
budget. The fast ticker still samples raw free space on every tick. It
uses only a cached estimate while a scan or removal is running.

The capacity unit tests count calls at the probe boundary using a fake
that exercises the production sampler. The native integration test is
ignored by default and uses one query. Run it explicitly with

```sh
cargo test -p disktree-core --test pwrdisk_capacity -- --ignored
```
