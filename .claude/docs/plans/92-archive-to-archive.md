# 92. From archive to archive — plan

Spec: `../specs/92-archive-to-archive.md`.

1. Core: `repack::cross` (tests: zip → tar into a dir; F6 inside one archive; a skipped conflict is not deleted).
2. `jobs::Job::Cross`.
3. `App::start_transfer`: the archive → archive arm builds it (same dir / into itself refused);
   `read_only` and the drop's early refusal go.
4. App tests: drop and paste between dirs of one archive start the job.
