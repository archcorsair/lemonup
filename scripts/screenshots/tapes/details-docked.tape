Source tapes/common.tape
Output shots/details-docked.txt

# Launch hidden; the addon list scans the fixture on start. At about 140
# columns (120 or more) Details docks on the right of the list.
Hide
Type "clear && ./lemonup"
Enter
Sleep 4s
# Deadly Boss Mods: Update ready, 12.0.8 to 12.0.10.
Down
Enter
Show
Sleep 1500ms
Screenshot shots/details-docked.png

Hide
Escape
Type "q"
Sleep 1s
