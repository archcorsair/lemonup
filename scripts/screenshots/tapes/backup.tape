Source tapes/common.tape
Output shots/backup.txt

# Launch hidden; the dashboard scans the fixture on start.
Hide
Type "clear && ./lemonup"
Enter
Sleep 4s
Type "b"
Show
Sleep 1500ms
Screenshot shots/backup.png

Hide
Escape
Type "q"
Sleep 1s
