Source tapes/common.tape
# A bigger font makes the terminal narrower than 120 columns, so Details
# opens as a sheet below the list instead of docked on the right.
Set FontSize 42
Output shots/details.txt

# Launch hidden; the addon list scans the fixture on start.
Hide
Type "clear && ./lemonup"
Enter
Sleep 4s
# HandyNotes: the source lists v1.6.14, older than the installed v1.6.15,
# so Details says "Newer than source".
Down 13
Enter
Show
Sleep 1500ms
Screenshot shots/details.png

Hide
Escape
Type "q"
Sleep 1s
