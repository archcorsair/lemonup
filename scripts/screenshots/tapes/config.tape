Source tapes/common.tape
Output shots/config.txt

# Launch hidden; the dashboard scans the fixture on start.
Hide
Type "clear && ./lemonup"
Enter
Sleep 4s
Type ","
Show
Sleep 1500ms
Screenshot shots/config.png

Hide
Escape
Type "q"
Sleep 1s
