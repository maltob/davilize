# Running scheduled reports

A report can declare its schedules (see [reports.md](reports.md#schedules)). `davilize schedule` runs them in one of two ways.

## Daemon

```
davilize schedule run report.dvz
davilize schedule run report.dvz --name weekday_refresh
davilize --json schedule run report.dvz
```

The daemon stays in the foreground. It sleeps until the next due schedule, runs it, and repeats. Each run reloads the report, so edits apply from the next run, and a failing run is logged without stopping the daemon. A run that overruns its time does not replay the missed times. Ctrl+C stops the daemon after the current run. Each run logs a start line and an ok or failed line with its duration; `--json` writes these as JSON lines on stderr.

## One run with --now

```
davilize schedule run report.dvz --now
davilize schedule run report.dvz --now --name weekday_refresh
```

`--now` runs the selected schedules once and exits: every enabled schedule without `--name`, or the named ones (also when disabled). It exits with `0` when all succeed, and otherwise with the exit code of the first failure (`3` input, `4` query, `5` output, `6` publish). A failure does not stop the other schedules. `--now` ignores the `cron` field, so the timing comes from the scheduler that calls it.

**Windows Task Scheduler** (weekdays at 07:30, running the report's `weekday_refresh` schedule):

```
schtasks /Create /TN "Davilize weekday refresh" /SC WEEKLY /D MON,TUE,WED,THU,FRI /ST 07:30 /TR "\"C:\tools\davilize.exe\" schedule run \"C:\reports\sales.dvz\" --now --name weekday_refresh"
```

**cron** (Linux or macOS, from `crontab -e`):

```
30 7 * * 1-5 /usr/local/bin/davilize schedule run /srv/reports/sales.dvz --now --name weekday_refresh >> /var/log/davilize.log 2>&1
```

**systemd timer**: in `/etc/systemd/system/davilize-sales.service`:

```
[Unit]
Description=Davilize sales refresh

[Service]
Type=oneshot
ExecStart=/usr/local/bin/davilize schedule run /srv/reports/sales.dvz --now
```

and in `/etc/systemd/system/davilize-sales.timer`:

```
[Unit]
Description=Run the sales refresh on weekdays at 07:30

[Timer]
OnCalendar=Mon..Fri *-*-* 07:30:00
Persistent=true

[Install]
WantedBy=timers.target
```

Then run `systemctl enable --now davilize-sales.timer`.

Scheduled publishes need their credentials in the environment of the task, for example `AWS_ACCESS_KEY_ID`, or the SFTP settings in [publishing.md](publishing.md#credentials). Scheduled runs write the same logs and exit codes as `refresh` and `publish`.

## Daemon or OS scheduler

The daemon keeps the timing in the report and needs no OS configuration, but it only runs while its process is alive, so after a reboot or a crash it must be started again, usually as a service. An OS scheduler such as Task Scheduler, cron or a systemd timer is the better fit for servers and unattended machines, because it survives reboots and holds the timing itself. Use one approach per report: a daemon and an OS task that run the same schedule at the same time would refresh the report twice.
