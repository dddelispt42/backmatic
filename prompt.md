I agree with all improvements. Please create a plan to implement all improvements. When splitting the project into library and binary,please do not create separate repo. I want all in one. 

Please also implement a complete set of unit tests. The unit test shall generate test files in a directory hierarchy and must at minimum cover verifications for:
- files being backed up
- incremental backups
- adding, modifying and removing files between backups and check the diffs between backups
- recovering files from a backup
- file retention (hourly/daily/weekly/…) and pruning. Remark: This might need some mocking or container where the system time can be changed as the pruning depends on the age of the backup. 
- verify file exclusion (simple, wildcard and globs)
- tests that the backup destination location is not part of the src location (which could lead to infinitely growing backup sizes)
- LUKS mounting/unmounting. Remark: This might require the creation of the file based loop filesystem (encrypted)
- test the database backups
The test cases must be run for all tools (i.e. rsync, borg, restic).

I also want additional features:

1. similar to the "destmount", I want a srcmount, which would mount the src directory from a remote machine (src) into a temporary directory and then backup that temporary directory. The idea is that it is not needed to run the backup from the src machine (untrusted) but from the backup/dest machine (trusted). Remark: I rather prefer to ssh/sftp from the backup machine to the many src machines than the other way around, since a compromised src machine would have access to the backup machine and could thus also compromise the backups (Must be avoided!). Just like the "dest" string/list field, the "destmount" should be allowed to be a single item or a list of items. Please investigate if it is better to have a single string (e.g. sftp://host.domain:port/dir/subdir) and parse the parameters (i.e. protocol/host/path) or to have multiple fields for those parameter. Please evaluate the better option and present arguments.

2. A just (Justfile) based wrapper for Cargo, which includes security scan/audits of the supply chain and which creates also a minimal docker image with the compiled backmatic binary, all external tools (e.g. rsync/borg/restic/…). The src root directory and destination root directory as well as the backmatic.yml config files are to be mounted as volumes (not part of the image/container).

3. The continuous mode (option: "--continuous") is unprecise. The option "-C 24" should e.g. start every day (24h) at the same time. With the current implementation there is a big drift, which make the backup start every day at different times. Please propose an improvement. 

4. The default for "--threads" parameter should be automatic derived from the number of CPU cores (divided by 2). Currently the application only moves from rsync to borg and/or from borg to restic once all previous jobs are successfully executed. If there is one job failing repeatedly, the next job type is not executed before all retries exceed the defined number. I want that the next backup job types starts once there is a thread available. Example: if there are 4 thread and from the 5 rsync backup already 2 terminated, then there is 1 thread (i.e. 4 - (5-2) = 1) for the borg backups. 

5. The backup jobs should have optional fields to integrate with healthchecks.io (the self-hosted version. Thus the URL, API token and the healthcheck specific UUID should be optional parameters per backup job. The HTTP request to the hearlthchecks.io server should be called whenever a job succeeded (i.e. report success) or when all the retries for a backup job failed (i.e. report failure). 

6. Provide a yaml schema for the backmatic config file (e.g. backmatic.yml) that can be loaded into the editor to ensure correctness of the configuration. This schema should also check for more dynamic rules - e.g. options not available for specific backup types. 

7. Extend the database backup to also cover PostgreSQL.

8. Dependency injection - i.e. no hardcoded paths, parameters, constants etc.

Please provide a detailed plan for me to review!

