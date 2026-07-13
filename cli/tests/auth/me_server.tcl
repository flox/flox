# Minimal mock of FloxHub's `GET /api/v1/accounts/me` for bats tests,
# run with `expect -f me_server.tcl` (expect ships a full Tcl interpreter,
# and Tcl has a native TCP server — no extra test dependencies needed).
#
# Environment:
#   MOCK_ME_SECRET     token secret expected in `Authorization: bearer <secret>`
#   MOCK_ME_HANDLE     handle returned on a match
#   MOCK_ME_EXPIRES_AT JSON value for expires_at, e.g. null or "2001-01-01T00:00:00Z"
#   MOCK_ME_PORT_FILE  file the bound port is written to once listening
#
# A wrong or missing token gets a 401; any other path gets a 404.

proc respond {chan status body} {
    puts $chan "HTTP/1.1 $status"
    puts $chan "content-type: application/json"
    puts $chan "content-length: [string length $body]"
    puts $chan "connection: close"
    puts $chan ""
    puts -nonewline $chan $body
    flush $chan
    close $chan
}

proc accept {chan addr port} {
    fconfigure $chan -translation crlf -buffering line
    set request [gets $chan]
    set auth ""
    while {[gets $chan line] >= 1} {
        regexp -nocase {^authorization:\s*(.*)$} $line -> auth
    }
    if {![regexp {^GET /api/v1/accounts/me } $request]} {
        respond $chan "404 Not Found" {{"detail":"not found"}}
        return
    }
    if {$auth ne "bearer $::env(MOCK_ME_SECRET)"} {
        respond $chan "401 Unauthorized" {{"detail":"unauthorized"}}
        return
    }
    respond $chan "200 OK" [format \
        {{"user_id":"pat|1","handle":"%s","expires_at":%s}} \
        $::env(MOCK_ME_HANDLE) $::env(MOCK_ME_EXPIRES_AT)]
}

set srv [socket -server accept -myaddr 127.0.0.1 0]
set port [lindex [fconfigure $srv -sockname] 2]
set f [open $::env(MOCK_ME_PORT_FILE) w]
puts $f $port
close $f
vwait forever
