# Windows builds for this fork

Run **Windows release (fork)** in GitHub Actions and choose a source tag or commit (default: `v0.7.1`). The workflow builds x64 on Windows using the repository's packaging script, without upstream reusable workflows or signing secrets.

The `pdfcraft-windows-x64` artifact contains an MSI installer, a portable ZIP with the GUI and CLI executables and OCR models, SHA-256 checksums, and the source commit. Packages are unsigned. The portable ZIP stores settings beside the executable in `PdfCraftData`.

Korean interface text uses fonts installed on Windows. The optional upstream craft-fonts collection is not bundled by this workflow; languages requiring that collection may have incomplete coverage.

Building an artifact does not publish a GitHub release or move an existing tag. Download and verify the artifact before attaching it to a release.
