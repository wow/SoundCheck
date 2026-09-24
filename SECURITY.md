# Security policy

SoundCheck reads and rewrites people's music files, so a bug that corrupts a file or leaks data is treated as a security issue.

## Supported versions

Only the latest release on the current minor line receives fixes. Pre-releases (`-alpha`, `-beta`, `-rc`) are not supported once the final release exists.

## Reporting

Please do not open a public issue. Use GitHub's private vulnerability reporting on this repository ("Security" tab, "Report a vulnerability"). Include the SoundCheck version, macOS version, the file format involved and, if possible, a small file that reproduces the problem.

You will get an acknowledgement within 48 hours. A fix or a mitigation ships within 7 days as a patch release, and the report is published as a GitHub security advisory with credit to the reporter, unless you prefer to stay anonymous.

## Scope

In scope: anything that makes SoundCheck write a wrong or truncated file, lose metadata, escape the folders the user chose, execute untrusted content, or expose local data. Out of scope: the third-party DJ software SoundCheck exports to.

## History

No advisories yet.
