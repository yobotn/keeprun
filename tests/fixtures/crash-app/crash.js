// Test app for Keeprun: prints a line and exits with code 1, to exercise the restart policy.
console.log("crash-app starting, will exit with code 1");
setTimeout(() => process.exit(1), 300);
